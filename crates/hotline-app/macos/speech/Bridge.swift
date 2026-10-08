// The C entry points the shell's `speech` module calls. Each takes the
// caller's context pointer and a reply callback, and calls that callback
// exactly once: the Rust side turns the pointer back into the channel that
// is waiting for the answer. Strings handed to a callback live only for the
// call. Recognition events go to the callback `start` was given, as JSON.
import Foundation

public typealias HotlineSpeechReply = @convention(c) (UnsafeMutableRawPointer?, Bool, UnsafePointer<CChar>?) -> Void
public typealias HotlineSpeechEvents = @convention(c) (UnsafePointer<CChar>) -> Void

private let controller = SpeechController()

/** A C caller's context crosses to the main actor untouched; only the reply reads it. */
private struct Reply: @unchecked Sendable {
  let context: UnsafeMutableRawPointer?
  let done: HotlineSpeechReply

  func send(_ ok: Bool, _ text: String? = nil) {
    guard let text else { return done(context, ok, nil) }
    text.withCString { done(context, ok, $0) }
  }
}

private struct Events: @unchecked Sendable {
  let send: HotlineSpeechEvents

  func emit(_ event: [String: Any]) {
    guard let json = json(event) else { return }
    json.withCString { send($0) }
  }
}

private func json(_ value: [String: Any]) -> String? {
  guard let data = try? JSONSerialization.data(withJSONObject: value) else { return nil }
  return String(data: data, encoding: .utf8)
}

/** Replies with the capability as JSON. */
@_cdecl("hotline_speech_capability")
public func hotlineSpeechCapability(_ context: UnsafeMutableRawPointer?, _ done: HotlineSpeechReply) {
  let reply = Reply(context: context, done: done)
  Task.detached {
    reply.send(true, json(await SpeechController.capability()))
  }
}

@_cdecl("hotline_speech_permit")
public func hotlineSpeechPermit(_ context: UnsafeMutableRawPointer?, _ done: HotlineSpeechReply) {
  let reply = Reply(context: context, done: done)
  Task { @MainActor in
    reply.send(await controller.permit())
  }
}

/** Replies false with no text when speech is unavailable, false with a message when starting failed. */
@_cdecl("hotline_speech_start")
public func hotlineSpeechStart(
  _ sessionId: UnsafePointer<CChar>, _ events: HotlineSpeechEvents,
  _ context: UnsafeMutableRawPointer?, _ done: HotlineSpeechReply
) {
  let id = String(cString: sessionId)
  let sink = Events(send: events)
  let reply = Reply(context: context, done: done)
  Task { @MainActor in
    do {
      reply.send(try await controller.start(sessionId: id) { event in sink.emit(event) })
    } catch is CancellationError {
      reply.send(false)
    } catch {
      reply.send(false, error.localizedDescription)
    }
  }
}

/** Replies true with the final text, or false with why there is none. A cancelled stop is empty text. */
@_cdecl("hotline_speech_stop")
public func hotlineSpeechStop(
  _ sessionId: UnsafePointer<CChar>, _ context: UnsafeMutableRawPointer?, _ done: HotlineSpeechReply
) {
  let id = String(cString: sessionId)
  let reply = Reply(context: context, done: done)
  Task { @MainActor in
    do {
      reply.send(true, try await controller.stop(sessionId: id))
    } catch is CancellationError {
      reply.send(true, "")
    } catch {
      reply.send(false, error.localizedDescription)
    }
  }
}

@_cdecl("hotline_speech_cancel")
public func hotlineSpeechCancel(
  _ sessionId: UnsafePointer<CChar>, _ context: UnsafeMutableRawPointer?, _ done: HotlineSpeechReply
) {
  let id = String(cString: sessionId)
  let reply = Reply(context: context, done: done)
  Task { @MainActor in
    controller.cancel(sessionId: id)
    reply.send(true)
  }
}
