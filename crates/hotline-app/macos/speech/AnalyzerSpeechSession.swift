// Xcode 26's SDK is required only for this preferred engine. Older SDKs build the local legacy path.
#if compiler(>=6.2)
import AVFoundation
import Foundation
import Speech

@available(macOS 26.0, *)
@MainActor
final class AnalyzerSpeechSession: LocalSpeechSession {
  private let locale: Locale
  private let emit: SpeechEmit
  private let microphone: SpeechMicrophone
  private var analyzer: SpeechAnalyzer?
  private var input: AsyncStream<AnalyzerInput>.Continuation?
  private var audio: AnalyzerAudioSink?
  private var results: Task<Void, Error>?
  private var finishing: Task<String, Error>?
  private var cancelled = false
  private var completed: String?
  private var failure: Error?
  private var committed: [(range: CMTimeRange, text: String)] = []
  private var volatile = ""

  init(locale: Locale, emit: @escaping SpeechEmit, microphone: SpeechMicrophone) {
    self.locale = locale
    self.emit = emit
    self.microphone = microphone
  }

  nonisolated static func installedLocale(equivalentTo locale: Locale) async -> Locale? {
    guard let supported = await supportedLocale(equivalentTo: locale) else { return nil }
    let installed = await SpeechTranscriber.installedLocales
    return installed.first { $0.identifier == supported.identifier }
  }

  nonisolated static func supportedLocale(equivalentTo locale: Locale) async -> Locale? {
    guard SpeechTranscriber.isAvailable else { return nil }
    return await SpeechTranscriber.supportedLocale(equivalentTo: locale)
  }

  private var text: String {
    // Final phrases arrive in order; replace the volatile phrase until the same range becomes final.
    (committed.map(\.text) + [volatile]).filter { !$0.isEmpty }.joined(separator: " ")
      .trimmingCharacters(in: .whitespacesAndNewlines)
  }

  func start() async throws {
    // Configure voice processing before the first format query: it can change the input's format.
    microphone.prepare()
    let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
    guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(
      compatibleWith: [transcriber], considering: microphone.format) else {
      throw LocalSpeechError(message: "This Mac's on-device speech model is not installed.")
    }
    guard !cancelled else { throw CancellationError() }
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    self.analyzer = analyzer
    try await analyzer.prepareToAnalyze(in: format)
    guard !cancelled else { await analyzer.cancelAndFinishNow(); throw CancellationError() }
    let (sequence, continuation) = AsyncStream<AnalyzerInput>.makeStream(bufferingPolicy: .bufferingOldest(64))
    input = continuation
    results = Task { [weak self] in
      do {
        for try await result in transcriber.results {
          guard let self, !self.cancelled else { return }
          let phrase = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
          if result.isFinal {
            if let existing = self.committed.firstIndex(where: { CMTimeRangeEqual($0.range, result.range) }) {
              self.committed[existing] = (result.range, phrase)
            } else {
              self.committed.append((result.range, phrase))
            }
            self.volatile = ""
          } else {
            self.volatile = phrase
          }
          self.emit(["type": "partial", "text": self.text])
        }
      } catch {
        self?.fail(error)
        throw error
      }
    }
    try await analyzer.start(inputSequence: sequence)
    guard !cancelled else { await analyzer.cancelAndFinishNow(); throw CancellationError() }
    let audio = try AnalyzerAudioSink(from: microphone.format, to: format, input: continuation) {
      [weak self] error in self?.fail(error)
    }
    self.audio = audio
    try microphone.start(
      owner: ObjectIdentifier(self),
      consume: { buffer in audio.append(buffer) },
      level: { [weak self] db in
        guard let self, !self.cancelled, self.finishing == nil else { return }
        self.emit(["type": "level", "levelDb": db, "unit": "dbfs", "at": Date().timeIntervalSince1970 * 1000])
      },
      interrupted: { [weak self] in
        self?.fail(LocalSpeechError(message: "The microphone changed. Call again."))
      })
  }

  func stop() async throws -> String {
    if let completed { return completed }
    if let failure { throw failure }
    if let finishing { return try await finishing.value }
    guard !cancelled, let analyzer else { throw CancellationError() }
    microphone.stop(owner: ObjectIdentifier(self))
    audio?.finish()
    audio = nil
    input = nil
    let results = self.results
    let task = Task<String, Error> { [weak self] in
      let timeout = Task {
        try? await Task.sleep(nanoseconds: 5_000_000_000)
        guard !Task.isCancelled else { return }
        self?.fail(LocalSpeechError(message: "On-device speech did not finish. Please try again."))
        await analyzer.cancelAndFinishNow()
      }
      defer { timeout.cancel() }
      try await analyzer.finalizeAndFinishThroughEndOfInput()
      try await results?.value
      guard let self, !self.cancelled else { throw CancellationError() }
      if let failure = self.failure { throw failure }
      let text = self.text
      self.completed = text
      self.emit(["type": "final", "text": text])
      self.emit(["type": "ended", "reason": text.isEmpty ? "no-speech" : "final"])
      return text
    }
    finishing = task
    do { return try await task.value } catch {
      if !cancelled { fail(error) }
      throw error
    }
  }

  private func fail(_ error: Error) {
    guard !cancelled, failure == nil, completed == nil else { return }
    failure = error
    microphone.stop(owner: ObjectIdentifier(self))
    audio?.finish()
    audio = nil
    input?.finish()
    input = nil
    if let analyzer { Task { await analyzer.cancelAndFinishNow() } }
    results?.cancel()
    emit(["type": "error", "message": error.localizedDescription])
    emit(["type": "ended", "reason": "error"])
  }

  func cancel() {
    guard !cancelled else { return }
    cancelled = true
    microphone.stop(owner: ObjectIdentifier(self))
    audio?.finish()
    audio = nil
    input?.finish()
    input = nil
    results?.cancel()
    finishing?.cancel()
    if let analyzer { Task { await analyzer.cancelAndFinishNow() } }
  }
}

/** Owns converter state on the audio thread, with a lock against stop/cancel on Main. */
@available(macOS 26.0, *)
private final class AnalyzerAudioSink: @unchecked Sendable {
  private let converter: AVAudioConverter
  private let format: AVAudioFormat
  private let input: AsyncStream<AnalyzerInput>.Continuation
  private let onFailure: @MainActor @Sendable (Error) -> Void
  private let lock = NSLock()
  private var accepting = true

  init(from source: AVAudioFormat, to target: AVAudioFormat,
    input: AsyncStream<AnalyzerInput>.Continuation,
    onFailure: @escaping @MainActor @Sendable (Error) -> Void
  ) throws {
    guard let converter = AVAudioConverter(from: source, to: target) else {
      throw LocalSpeechError(message: "This microphone cannot supply on-device speech audio.")
    }
    converter.primeMethod = .none
    self.converter = converter
    self.format = target
    self.input = input
    self.onFailure = onFailure
  }

  func append(_ buffer: AVAudioPCMBuffer) {
    lock.lock()
    defer { lock.unlock() }
    guard accepting else { return }
    let source = converter.inputFormat
    guard buffer.format.sampleRate == source.sampleRate,
      buffer.format.channelCount == source.channelCount,
      buffer.format.commonFormat == source.commonFormat,
      buffer.format.isInterleaved == source.isInterleaved else {
      fail(LocalSpeechError(message: "The microphone changed its audio format. Call again."))
      return
    }
    let frames = AVAudioFrameCount(ceil(Double(buffer.frameLength) * format.sampleRate / buffer.format.sampleRate)) + 32
    guard let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames) else {
      fail(LocalSpeechError(message: "The microphone could not allocate speech audio."))
      return
    }
    var error: NSError?
    var supplied = false
    let status = converter.convert(to: converted, error: &error) { _, state in
      if supplied { state.pointee = .noDataNow; return nil }
      supplied = true
      state.pointee = .haveData
      return buffer
    }
    if status == .error || error != nil {
      fail(error ?? NSError(domain: "HotlineSpeech", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Microphone conversion failed."]))
      return
    }
    guard converted.frameLength > 0 else { return }
    // Each converted buffer is freshly allocated; never enqueue the tap's reused buffer.
    if case .dropped = input.yield(AnalyzerInput(buffer: converted)) {
      fail(LocalSpeechError(message: "On-device speech could not keep up. Please try again."))
    }
  }

  private func fail(_ error: Error) {
    accepting = false
    let onFailure = self.onFailure
    Task { @MainActor in onFailure(error) }
  }

  func finish() {
    lock.lock()
    defer { lock.unlock() }
    accepting = false
    input.finish()
  }
}
#endif
