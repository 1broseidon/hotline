// Feeds recorded speech through the analyzer session's audio sink the way
// the microphone tap does (`check/run` says how to run it). Voice processing
// gives the tap the processed voice first, then one reference channel per
// output channel carrying what the Mac plays: one or two channels for most,
// three for a Bluetooth headset playing stereo. Every count must reach the
// analyzer as the same words, and the meter must read the voice alone.
// With no microphone connected it also starts a session, which must refuse
// with the window's words rather than abort. Last, it plays a teammate's
// reply into the reference channels and 30 dB down into the voice channel,
// as echo the canceller left: the analyzer must hear nothing but the reply's
// own words there (the window takes those for echo, never the person), and
// must hear a person talking over it.
import AVFoundation
import CoreAudio
import Foundation
import Speech

@main
struct Check {
  static func main() async {
    let arguments = CommandLine.arguments
    guard arguments.count > 4 else {
      print("usage: check <clip> <reply> <over-reply> <word>...")
      exit(2)
    }
    guard #available(macOS 26.0, *) else {
      print("skipped: SpeechAnalyzer needs macOS 26")
      exit(0)
    }
    let clip = URL(fileURLWithPath: arguments[1])
    let reply = URL(fileURLWithPath: arguments[2])
    let over = URL(fileURLWithPath: arguments[3])
    let words = arguments.dropFirst(4).map { $0.lowercased() }
    var failed = !meterHearsTheVoiceAlone()
    failed = !inputChangesAreNamed() || failed
    failed = !(await noMicrophoneRefuses()) || failed
    for channels in 1...3 {
      do {
        let heard = try await hear(clip, channels: channels)
        let spoken = heard.text.lowercased().filter { $0.isLetter || $0 == " " }
          .split(separator: " ").map(String.init)
        let missing = words.filter { !spoken.contains($0) }
        let ok = missing.isEmpty && heard.silent == 0
        failed = failed || !ok
        print("\(ok ? "ok  " : "FAIL") \(channels) ch: \(heard.buffers) buffers, \(heard.silent) silent, heard \"\(heard.text)\"",
          missing.isEmpty ? "" : "missing \(missing)")
      } catch {
        failed = true
        print("FAIL \(channels) ch: \(error.localizedDescription)")
      }
    }
    do { failed = !(try await echoIsTheReplyAndThePersonIsHeard(reply: reply, over: over)) || failed } catch {
      failed = true
      print("FAIL reply leaking into the microphone: \(error.localizedDescription)")
    }
    exit(failed ? 1 : 0)
  }

  /** Lower case letters, as the window compares words (ui/src/voice/bargeIn.ts). */
  static func words(_ text: String) -> [String] {
    text.lowercased().split(whereSeparator: { $0 == " " }).map { $0.filter { $0.isLetter || $0.isNumber } }
      .filter { !$0.isEmpty }
  }

  /** The window's loose match: the engine writes "look" for a reply's "looked". */
  static func sameWord(_ heard: String, _ said: String) -> Bool {
    heard == said || (heard.count >= 3 && said.count >= 3 && (said.hasPrefix(heard) || heard.hasPrefix(said)))
  }

  /**
   * A reply playing to a headset, its sound in the two reference channels
   * and, 30 dB down, in the voice channel as residual echo; then a person
   * talking over it from 1.5 s. The echo alone must give only the reply's
   * words; over it, the person's must be heard.
   */
  @available(macOS 26.0, *)
  static func echoIsTheReplyAndThePersonIsHeard(reply: URL, over: URL) async throws -> Bool {
    let rate = 24_000.0
    let played = try monoSamples(reply, rate: rate)
    let person = try monoSamples(over, rate: rate)
    let script = words(try String(contentsOf: reply.deletingPathExtension().appendingPathExtension("txt"), encoding: .utf8))
    let leak: Float = 0.0316
    let echo = try await hear(frames: played.count, channels: 3) { channel, n in
      channel == 0 ? leak * played[n] + Float.random(in: -0.0005...0.0005) : played[n]
    }
    let foreign = words(echo.text).filter { heard in !script.contains { sameWord(heard, $0) } }
    let start = Int(1.5 * rate)
    let both = try await hear(frames: max(played.count, start + person.count), channels: 3) { channel, n in
      let reply = n < played.count ? played[n] : 0
      if channel > 0 { return reply }
      let voice = n >= start && n - start < person.count ? person[n - start] : 0
      return voice + leak * reply + Float.random(in: -0.0005...0.0005)
    }
    let heard = words(both.text)
    let missing = ["stop", "machine"].filter { !heard.contains($0) }
    return [
      report(foreign.isEmpty, "reply leaking 30 dB down: \(words(echo.text).count) words, \(foreign.isEmpty ? "all the reply's own" : "not the reply's: \(foreign)")"),
      report(missing.isEmpty, "person over the reply: heard \"\(both.text)\"" + (missing.isEmpty ? "" : ", missing \(missing)")),
    ].allSatisfy { $0 }
  }

  /** Prints one line for a check and answers whether it passed. */
  static func report(_ ok: Bool, _ what: String) -> Bool {
    print("\(ok ? "ok  " : "FAIL") \(what)")
    return ok
  }

  /**
   * A three-channel tap buffer as voice processing gives it: the processed
   * microphone, then the reference channels carrying the reply that plays.
   */
  static func tapBuffer(voice: Float, reference: Float, channels: Int = 3) -> AVAudioPCMBuffer {
    let layout = AVAudioChannelLayout(layoutTag: kAudioChannelLayoutTag_DiscreteInOrder | UInt32(channels))!
    let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 24_000, interleaved: false, channelLayout: layout)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1_024)!
    buffer.frameLength = 1_024
    for frame in 0..<1_024 {
      let tone = sin(Float(frame) * 2 * .pi * 220 / 24_000)
      // A quiet room is never exactly zero.
      buffer.floatChannelData![0][frame] = voice * tone + Float.random(in: -0.0002...0.0002)
      for channel in 1..<channels { buffer.floatChannelData![channel][frame] = reference * tone }
    }
    return buffer
  }

  /** The meter reads the first channel: a reply in the reference channels is not the person speaking. */
  static func meterHearsTheVoiceAlone() -> Bool {
    let replyPlaying = SpeechMicrophone.voiceLevel(tapBuffer(voice: 0, reference: 0.3)) ?? 0
    let personSpeaking = SpeechMicrophone.voiceLevel(tapBuffer(voice: 0.3, reference: 0)) ?? -160
    let one = SpeechMicrophone.voiceLevel(tapBuffer(voice: 0.3, reference: 0, channels: 1)) ?? -160
    return [
      report(replyPlaying < -60, String(format: "meter, reply in the reference channels only: %.0f dBFS", replyPlaying)),
      report(personSpeaking > -20, String(format: "meter, voice in the first channel: %.0f dBFS", personSpeaking)),
      report(abs(one - personSpeaking) < 1, String(format: "meter, one channel reads as three: %.0f dBFS", one)),
    ].allSatisfy { $0 }
  }

  /** A device gone is told from a device changed, and voice processing settling is neither. */
  static func inputChangesAreNamed() -> Bool {
    let cases: [(AudioDeviceID?, AudioDeviceID?, Int, Bool, SpeechMicrophone.InputChange)] = [
      (7, 7, 1, true, .same),
      (7, 7, 1, false, .changed),
      (7, 9, 1, true, .changed),
      (7, nil, 0, true, .gone),
      (7, 7, 0, true, .gone),
    ]
    return cases.map { start, now, channels, sameFormat, expected in
      let change = SpeechMicrophone.inputChange(from: start, to: now, channels: channels, sameFormat: sameFormat)
      return report(change == expected, "configuration change \(start.map(String.init) ?? "none") -> \(now.map(String.init) ?? "none"), \(channels) ch: \(change)")
    }.allSatisfy { $0 }
  }

  /**
   * With no microphone connected (a Mac Studio with its headset away), a
   * session refuses with the window's words. Before the guard the tap
   * aborted the process: "Failed to create tap due to format mismatch".
   * Skipped while a microphone is connected, so the check never opens one.
   */
  @MainActor
  static func noMicrophoneRefuses() async -> Bool {
    guard SpeechMicrophone.inputChannels() == 0 else {
      print("skip no-microphone start: a microphone is connected")
      return true
    }
    let microphone = SpeechMicrophone()
    await microphone.warmUp()
    var results = [report(true, "no microphone: warming up returns")]
    do {
      try microphone.start(owner: ObjectIdentifier(microphone), consume: { _ in }, level: { _ in }, interrupted: { _ in })
      results.append(report(false, "no microphone: the microphone started"))
    } catch {
      let said = error.localizedDescription
      results.append(report(said == LocalSpeechError.noMicrophone.message, "no microphone: refused, \"\(said)\""))
    }
    if #available(macOS 26.0, *),
      let locale = await AnalyzerSpeechSession.installedLocale(equivalentTo: Locale(identifier: "en_US"))
    {
      microphone.prepare()
      let session = AnalyzerSpeechSession(locale: locale, emit: { _ in }, microphone: microphone)
      do {
        try await session.start()
        results.append(report(false, "no microphone: the analyzer session started"))
      } catch {
        let said = error.localizedDescription
        results.append(report(said == LocalSpeechError.noMicrophone.message, "no microphone: session refused, \"\(said)\""))
      }
      session.cancel()
    }
    return results.allSatisfy { $0 }
  }

  /** The clip as the tap would give it at 24 kHz with `channels` channels, through the sink, to the analyzer. */
  @available(macOS 26.0, *)
  static func hear(_ clip: URL, channels: Int) async throws -> (text: String, buffers: Int, silent: Int) {
    let voice = try monoSamples(clip, rate: 24_000)
    return try await hear(frames: voice.count, channels: channels) { _, n in voice[n] }
  }

  /** `frames` of 24 kHz audio, each channel's sample from `sample`, through the sink to the analyzer. */
  @available(macOS 26.0, *)
  static func hear(frames total: Int, channels: Int, sample: (Int, Int) -> Float) async throws
    -> (text: String, buffers: Int, silent: Int)
  {
    guard let locale = await AnalyzerSpeechSession.installedLocale(equivalentTo: Locale(identifier: "en_US")) else {
      throw LocalSpeechError(message: "the English speech model is not installed on this Mac")
    }
    let rate = 24_000.0
    // Discrete channels in order, the layout the processed tap reports.
    let layout = AVAudioChannelLayout(layoutTag: kAudioChannelLayoutTag_DiscreteInOrder | UInt32(channels))!
    let tap = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, interleaved: false, channelLayout: layout)
    let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
    guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber], considering: tap) else {
      throw LocalSpeechError(message: "no analyzer format")
    }
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    try await analyzer.prepareToAnalyze(in: format)
    let (sequence, input) = AsyncStream<AnalyzerInput>.makeStream(bufferingPolicy: .bufferingOldest(64))
    let results = Task {
      var text: [String] = []
      for try await result in transcriber.results where result.isFinal {
        text.append(String(result.text.characters))
      }
      return text.joined(separator: " ")
    }
    try await analyzer.start(inputSequence: sequence)
    let sink = try AnalyzerAudioSink(from: tap, to: format, input: input) { error in
      print("sink failed: \(error.localizedDescription)")
    }
    // 100 ms buffers, as the tap delivers them, a little faster than real time.
    let block = 2_400
    var offset = 0
    while offset < total {
      let frames = min(block, total - offset)
      let buffer = AVAudioPCMBuffer(pcmFormat: tap, frameCapacity: AVAudioFrameCount(frames))!
      buffer.frameLength = AVAudioFrameCount(frames)
      for channel in 0..<channels {
        for frame in 0..<frames { buffer.floatChannelData![channel][frame] = sample(channel, offset + frame) }
      }
      sink.append(buffer)
      offset += frames
      try await Task.sleep(nanoseconds: 50_000_000)
    }
    sink.finish()
    try await analyzer.finalizeAndFinishThroughEndOfInput()
    let text = try await results.value
    let sent = sink.sent
    return (text, sent.buffers, sent.silent)
  }

  /** The clip as mono float samples at `rate`. */
  static func monoSamples(_ clip: URL, rate: Double) throws -> [Float] {
    let file = try AVAudioFile(forReading: clip)
    let mono = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false)!
    let whole = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length))!
    try file.read(into: whole)
    let converter = AVAudioConverter(from: file.processingFormat, to: mono)!
    let capacity = AVAudioFrameCount(Double(file.length) * rate / file.processingFormat.sampleRate) + 1_024
    let out = AVAudioPCMBuffer(pcmFormat: mono, frameCapacity: capacity)!
    var supplied = false
    var error: NSError?
    converter.convert(to: out, error: &error) { _, state in
      if supplied { state.pointee = .endOfStream; return nil }
      supplied = true
      state.pointee = .haveData
      return whole
    }
    if let error { throw error }
    // A second of quiet room after the words, so the last phrase ends before the input does.
    return Array(UnsafeBufferPointer(start: out.floatChannelData![0], count: Int(out.frameLength)))
      + (0..<Int(rate)).map { _ in Float.random(in: -0.0005...0.0005) }
  }
}
