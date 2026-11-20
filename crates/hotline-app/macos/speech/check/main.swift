// Feeds recorded speech through the analyzer session's audio sink the way
// the microphone tap does (`check/run` says how to run it). Voice processing
// gives the tap its voice in every channel, one channel per channel of the
// device: one or two for most, three for a Bluetooth headset that was
// playing stereo when processing began. Every count must reach the
// analyzer as the same words.
import AVFoundation
import Foundation
import Speech

@main
struct Check {
  static func main() async {
    let arguments = CommandLine.arguments
    guard arguments.count > 2 else {
      print("usage: check <clip> <word>...")
      exit(2)
    }
    guard #available(macOS 26.0, *) else {
      print("skipped: SpeechAnalyzer needs macOS 26")
      exit(0)
    }
    let clip = URL(fileURLWithPath: arguments[1])
    let words = arguments.dropFirst(2).map { $0.lowercased() }
    var failed = false
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
    exit(failed ? 1 : 0)
  }

  /** The clip as the tap would give it at 24 kHz with `channels` channels, through the sink, to the analyzer. */
  @available(macOS 26.0, *)
  static func hear(_ clip: URL, channels: Int) async throws -> (text: String, buffers: Int, silent: Int) {
    guard let locale = await AnalyzerSpeechSession.installedLocale(equivalentTo: Locale(identifier: "en_US")) else {
      throw LocalSpeechError(message: "the English speech model is not installed on this Mac")
    }
    let rate = 24_000.0
    let voice = try monoSamples(clip, rate: rate)
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
    while offset < voice.count {
      let frames = min(block, voice.count - offset)
      let buffer = AVAudioPCMBuffer(pcmFormat: tap, frameCapacity: AVAudioFrameCount(frames))!
      buffer.frameLength = AVAudioFrameCount(frames)
      for channel in 0..<channels {
        voice.withUnsafeBufferPointer { samples in
          buffer.floatChannelData![channel].update(from: samples.baseAddress! + offset, count: frames)
        }
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
