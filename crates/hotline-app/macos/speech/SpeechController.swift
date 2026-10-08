// Per-utterance speech on this Mac, ported from the phone's HotlineSpeech
// module. Recognition never leaves the machine: the preferred engine is
// SpeechAnalyzer (macOS 26), the fallback SFSpeechRecognizer with on-device
// recognition required, and no network recognizer is ever constructed.
import AVFoundation
import CoreAudio
import Foundation
import Speech

struct LocalSpeechError: LocalizedError {
  let message: String
  var errorDescription: String? { message }
}

typealias SpeechEmit = @MainActor ([String: Any]) -> Void

@MainActor
protocol LocalSpeechSession: AnyObject {
  func start() async throws
  func stop() async throws -> String
  func cancel()
}

/** All mutable recognition state is used on the main actor. No remote recognizer is constructed. */
@MainActor
final class SpeechController {
  private var sessionId: String?
  private var session: (any LocalSpeechSession)?
  private var generation = 0
  private var cancelPreparation: (@MainActor () -> Void)?

  nonisolated init() {}

  /** Reads only: no permission prompt, no download, no microphone. */
  nonisolated static func capability() async -> [String: Any] {
    let locale = Locale.current
    #if compiler(>=6.2)
    if #available(macOS 26.0, *),
      let supported = await AnalyzerSpeechSession.supportedLocale(equivalentTo: locale)
    {
      let installed = await AnalyzerSpeechSession.installedLocale(equivalentTo: supported) != nil
      return ["available": true, "onDevice": true, "engine": "apple-analyzer", "locale": supported.identifier,
        "modelInstalled": installed]
    }
    #endif
    if let recognizer = SFSpeechRecognizer(locale: locale),
      recognizer.isAvailable, recognizer.supportsOnDeviceRecognition
    {
      return ["available": true, "onDevice": true, "engine": "apple-recognizer", "locale": locale.identifier]
    }
    return ["available": false, "onDevice": true, "locale": locale.identifier,
      "reason": "On-device speech is not installed or supported for this Mac's language."]
  }

  func permit() async -> Bool {
    let token = generation
    // Info.plist carries both usage descriptions; without them the system ends the process here.
    let speechAllowed = await withCheckedContinuation { continuation in
      SFSpeechRecognizer.requestAuthorization { status in
        continuation.resume(returning: status == .authorized)
      }
    }
    guard speechAllowed else { return false }
    let microphoneAllowed = await withCheckedContinuation { continuation in
      AVCaptureDevice.requestAccess(for: .audio) { allowed in
        continuation.resume(returning: allowed)
      }
    }
    guard microphoneAllowed, token == generation else { return false }
    #if compiler(>=6.2)
    if #available(macOS 26.0, *),
      let locale = await AnalyzerSpeechSession.supportedLocale(equivalentTo: .current)
    {
      guard token == generation else { return false }
      if await prepareAnalyzer(locale: locale, token: token) { return token == generation }
    }
    #endif
    // A model preparation failure can use the older local engine; never enable a network recognizer.
    guard token == generation, let recognizer = SFSpeechRecognizer(locale: .current) else { return false }
    return recognizer.isAvailable && recognizer.supportsOnDeviceRecognition
  }

  #if compiler(>=6.2)
  @available(macOS 26.0, *)
  private func prepareAnalyzer(locale: Locale, token: Int) async -> Bool {
    if await AnalyzerSpeechSession.installedLocale(equivalentTo: locale) != nil { return true }
    do {
      let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
      guard let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) else {
        return await AnalyzerSpeechSession.installedLocale(equivalentTo: locale) != nil
      }
      guard token == generation else { return false }
      return await withCheckedContinuation { continuation in
        var settled = false
        var download: Task<Void, Never>?
        var timeout: Task<Void, Never>?
        @MainActor func finish(_ ready: Bool) {
          guard !settled else { return }
          settled = true
          timeout?.cancel()
          self.cancelPreparation = nil
          continuation.resume(returning: ready)
        }
        self.cancelPreparation = { @MainActor in
          request.progress.cancel()
          download?.cancel()
          finish(false)
        }
        timeout = Task { @MainActor in
          try? await Task.sleep(nanoseconds: 60_000_000_000)
          guard !Task.isCancelled else { return }
          request.progress.cancel()
          download?.cancel()
          finish(false)
        }
        download = Task { @MainActor in
          do {
            try await request.downloadAndInstall()
            let ready = await AnalyzerSpeechSession.installedLocale(equivalentTo: locale) != nil
            finish(ready && token == self.generation)
          } catch { finish(false) }
        }
      }
    } catch { return false }
  }
  #endif

  func start(sessionId id: String, emit: @escaping SpeechEmit) async throws -> Bool {
    cancel()
    let token = generation
    sessionId = id // Cancellation must invalidate capability/preparation before a mic exists.
    guard SFSpeechRecognizer.authorizationStatus() == .authorized,
      AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else { return false }
    let locale = Locale.current
    guard token == generation else { return false }
    let event: SpeechEmit = { value in
      guard self.generation == token, self.sessionId == id else { return }
      var payload = value
      payload["sessionId"] = id
      emit(payload)
    }
    let next: any LocalSpeechSession
    #if compiler(>=6.2)
    if #available(macOS 26.0, *),
      let installed = await AnalyzerSpeechSession.installedLocale(equivalentTo: locale)
    {
      guard token == generation else { return false }
      next = AnalyzerSpeechSession(locale: installed, emit: event)
    } else {
      next = try LegacySpeechSession(locale: locale, emit: event)
    }
    #else
    next = try LegacySpeechSession(locale: locale, emit: event)
    #endif
    session = next
    do {
      try await next.start()
      guard token == generation else { next.cancel(); return false }
      return true
    } catch {
      next.cancel()
      if token == generation { session = nil; sessionId = nil }
      if token != generation { return false }
      throw error
    }
  }

  func stop(sessionId id: String) async throws -> String {
    guard id == sessionId, let current = session else { return "" }
    let token = generation
    let text = try await current.stop()
    guard token == generation else { return "" }
    session = nil
    sessionId = nil
    return text
  }

  /** The empty ID cancels whatever is current, including permission-time model preparation. */
  func cancel(sessionId id: String? = nil) {
    if let id, !id.isEmpty, id != sessionId { return }
    generation += 1
    cancelPreparation?()
    cancelPreparation = nil
    session?.cancel()
    session = nil
    sessionId = nil
  }
}

/**
 An input tap is removed before finishing recognition, so no cue can enter a closed utterance.
 A Mac has no audio session to configure: the engine's input node is the default input device.
 */
@MainActor
final class SpeechMicrophone {
  private let engine = AVAudioEngine()
  private var tapped = false
  private var configurationChange: NSObjectProtocol?
  private var inputPrepared = false
  /** The system's input device when the utterance began. */
  private var device: AudioDeviceID?

  var format: AVAudioFormat { engine.inputNode.outputFormat(forBus: 0) }

  func prepare() {
    // Analyzer calls prepare again when starting its tap. Do not retry a
    // rejected device then and change the format its converter already chose.
    guard !inputPrepared else { return }
    inputPrepared = true
    // Configure processing before SpeechAnalyzer chooses its converter format:
    // enabling it can change the input node's sample rate and channel count.
    let input = engine.inputNode
    do {
      if !input.isVoiceProcessingEnabled {
        try input.setVoiceProcessingEnabled(true)
      }
    } catch {
      // Some input devices cannot supply voice processing. Keep local
      // recognition available with their raw input.
      return
    }
    if #available(macOS 14.0, *) {
      // Processing ducks every other sound on the Mac by default, and the
      // call's spoken replies play from the window, another process's audio.
      input.voiceProcessingOtherAudioDuckingConfiguration =
        AVAudioVoiceProcessingOtherAudioDuckingConfiguration(enableAdvancedDucking: false, duckingLevel: .min)
    }
  }

  func start(
    consume: @escaping @Sendable (AVAudioPCMBuffer) -> Void,
    level: @escaping @MainActor @Sendable (Double) -> Void,
    interrupted: @escaping @MainActor @Sendable () -> Void
  ) throws {
    prepare()
    let input = engine.inputNode
    let format = input.outputFormat(forBus: 0)
    guard format.sampleRate > 0, format.channelCount > 0 else {
      throw LocalSpeechError(message: "This Mac's microphone is unavailable.")
    }
    let tap: @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void = { buffer, _ in
      consume(buffer)
      // Copy only the scalar into the actor; AVAudioEngine reuses the tap's buffer.
      if let db = Self.level(buffer) {
        Task { @MainActor in level(db) }
      }
    }
    input.installTap(onBus: 0, bufferSize: 1024, format: format, block: tap)
    tapped = true
    device = Self.defaultInputDevice()
    // The engine reports a configuration change for its own voice processing
    // settling as well as for a new device. Same device and format: start it
    // again and keep listening. A different device stops the engine with a
    // converter already fixed to the old format, so the utterance ends rather
    // than going quietly deaf.
    configurationChange = NotificationCenter.default.addObserver(
      forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
    ) { [weak self] _ in
      Task { @MainActor in
        guard let self else { return }
        let now = self.engine.inputNode.outputFormat(forBus: 0)
        let same = Self.defaultInputDevice() == self.device && now.isEqual(format)
        NSLog("[hotline-speech] configuration change: same device and format %@, running %@",
          same ? "yes" : "no", self.engine.isRunning ? "yes" : "no")
        if same {
          if self.engine.isRunning { return }
          do { try self.engine.start(); return } catch {}
        }
        interrupted()
      }
    }
    engine.prepare()
    do { try engine.start() } catch { stop(); throw error }
  }

  func stop() {
    if let configurationChange { NotificationCenter.default.removeObserver(configurationChange) }
    configurationChange = nil
    engine.stop()
    if tapped { engine.inputNode.removeTap(onBus: 0); tapped = false }
  }

  /** The device macOS records from now, or nil if it cannot say. */
  nonisolated private static func defaultInputDevice() -> AudioDeviceID? {
    var id = AudioDeviceID(0)
    var size = UInt32(MemoryLayout<AudioDeviceID>.size)
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyDefaultInputDevice,
      mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id)
    return status == noErr ? id : nil
  }

  nonisolated private static func level(_ buffer: AVAudioPCMBuffer) -> Double? {
    guard buffer.frameLength > 0, buffer.format.channelCount > 0 else { return nil }
    let channels = Int(buffer.format.channelCount)
    let frames = Int(buffer.frameLength)
    let interleaved = buffer.format.isInterleaved
    var power: Double = 0
    for channel in 0..<channels {
      for frame in 0..<frames {
        let pointer = interleaved ? 0 : channel
        let index = interleaved ? frame * channels + channel : frame
        let value: Double
        switch buffer.format.commonFormat {
        case .pcmFormatFloat32:
          guard let data = buffer.floatChannelData else { return nil }
          value = Double(data[pointer][index])
        case .pcmFormatInt16:
          guard let data = buffer.int16ChannelData else { return nil }
          value = Double(data[pointer][index]) / 32768
        case .pcmFormatInt32:
          guard let data = buffer.int32ChannelData else { return nil }
          value = Double(data[pointer][index]) / 2147483648
        default:
          return nil
        }
        power += value * value
      }
    }
    guard power.isFinite else { return nil }
    let mean = power / Double(frames * channels)
    return max(-160, min(0, 10 * log10(max(mean, 1e-16))))
  }
}
