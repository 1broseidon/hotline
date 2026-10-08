import AVFoundation
import Foundation
import Speech

@MainActor
final class LegacySpeechSession: LocalSpeechSession {
  private let recognizer: SFSpeechRecognizer
  private let emit: SpeechEmit
  private let microphone = SpeechMicrophone()
  private var request: SFSpeechAudioBufferRecognitionRequest?
  private var audio: LegacyAudioSink?
  private var task: SFSpeechRecognitionTask?
  private var waiter: CheckedContinuation<String, Error>?
  private var timeout: Task<Void, Never>?
  private var result: Result<String, Error>?
  private var cancelled = false
  private var text = ""

  init(locale: Locale, emit: @escaping SpeechEmit) throws {
    guard let recognizer = SFSpeechRecognizer(locale: locale),
      recognizer.isAvailable, recognizer.supportsOnDeviceRecognition else {
      throw LocalSpeechError(message: "On-device speech is unavailable for this language.")
    }
    self.recognizer = recognizer
    self.emit = emit
  }

  func start() async throws {
    let request = SFSpeechAudioBufferRecognitionRequest()
    request.requiresOnDeviceRecognition = true
    request.shouldReportPartialResults = true
    request.taskHint = .dictation
    request.addsPunctuation = true
    self.request = request
    let audio = LegacyAudioSink(request: request)
    self.audio = audio
    let onResult: @Sendable (SFSpeechRecognitionResult?, Error?) -> Void = { [weak self] value, error in
      // Snapshot immutable scalar values before hopping; no native result object crosses actors.
      let text = value?.bestTranscription.formattedString
      let final = value?.isFinal == true
      Task { @MainActor in
        guard let self, !self.cancelled, self.result == nil else { return }
        if let text {
          self.text = text
          if final {
            self.complete(.success(self.text))
            return
          }
          self.emit(["type": "partial", "text": self.text])
        }
        if let error { self.complete(.failure(error)) }
      }
    }
    task = recognizer.recognitionTask(with: request, resultHandler: onResult)
    try microphone.start(
      consume: { buffer in audio.append(buffer) },
      level: { [weak self] db in self?.level(db) },
      interrupted: { [weak self] in
        self?.complete(.failure(LocalSpeechError(message: "The microphone changed. Call again.")))
      })
  }

  private func level(_ db: Double) {
    guard !cancelled, result == nil else { return }
    emit(["type": "level", "levelDb": db, "unit": "dbfs", "at": Date().timeIntervalSince1970 * 1000])
  }

  func stop() async throws -> String {
    if let result { return try result.get() }
    guard !cancelled else { throw CancellationError() }
    microphone.stop()
    audio?.finish()
    return try await withCheckedThrowingContinuation { continuation in
      waiter = continuation
      timeout = Task { [weak self] in
        try? await Task.sleep(nanoseconds: 5_000_000_000)
        guard !Task.isCancelled else { return }
        self?.complete(.failure(LocalSpeechError(message: "On-device speech did not finish. Please try again.")))
      }
    }
  }

  private func complete(_ value: Result<String, Error>) {
    guard !cancelled, result == nil else { return }
    result = value
    microphone.stop()
    timeout?.cancel()
    timeout = nil
    request = nil
    audio?.finish()
    audio = nil
    switch value {
    case .success(let text):
      emit(["type": "final", "text": text])
      emit(["type": "ended", "reason": text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "no-speech" : "final"])
    case .failure(let error):
      task?.cancel()
      emit(["type": "error", "message": error.localizedDescription, "code": String((error as NSError).code)])
      emit(["type": "ended", "reason": "error"])
    }
    task = nil
    waiter?.resume(with: value)
    waiter = nil
  }

  func cancel() {
    guard !cancelled else { return }
    cancelled = true
    microphone.stop()
    timeout?.cancel()
    timeout = nil
    audio?.finish()
    audio = nil
    request = nil
    task?.cancel()
    task = nil
    waiter?.resume(throwing: CancellationError())
    waiter = nil
  }
}

/** The tap runs off Main. Serialize append/endAudio and never let that callback inherit an actor. */
private final class LegacyAudioSink: @unchecked Sendable {
  private let request: SFSpeechAudioBufferRecognitionRequest
  private let lock = NSLock()
  private var accepting = true

  init(request: SFSpeechAudioBufferRecognitionRequest) { self.request = request }

  func append(_ buffer: AVAudioPCMBuffer) {
    lock.lock()
    defer { lock.unlock() }
    if accepting { request.append(buffer) }
  }

  func finish() {
    lock.lock()
    defer { lock.unlock() }
    guard accepting else { return }
    accepting = false
    request.endAudio()
  }
}
