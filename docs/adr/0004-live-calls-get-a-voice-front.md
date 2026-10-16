# ADR 0004: Live calls get a voice front

- Status: Accepted (0.38); its guardrails for direct calls superseded by
  [ADR 0005](0005-front-is-a-full-agent.md)
- Date: 2026-10-14
- Supersedes: "No front" in [voice-calls.md](../voice-calls.md) (Direct turns)
- Design: [voice-calls.md](../voice-calls.md), [voice.md](../voice.md)
- Tracking: epic-6

## Context

A direct call today has no front. Every utterance is a turn of the teammate's
own session, and the teammate answers twice, `<spoken>` then `<written>`.
That kept one author for what is said and what is done, but the person waits
for a whole model turn before hearing anything, speech is a pipeline of
transcribe, think, synthesize, and talking over the teammate only stops the
audio: the front of the call cannot yield the floor, acknowledge, or ask
"which file?" while the work goes on.

Codex has the precedent: its realtime voice mode puts a speech-to-speech model
in front of the coding agent. The realtime model holds the conversation and
hands the work to the agent, which stays the one that does it.

## Decision

A live call gets a **voice front**: a realtime speech-to-speech model on the
call that hears the person, speaks to them, and relays to the teammate.

Why:

- **Latency.** A realtime model answers in the time a person expects from a
  voice, not after the teammate's turn. It can say it is on it at once.
- **Barge-in.** The front hears the person over its own speech and stops
  talking at the model, not by a client heuristic after the fact.
- **Subscriptions cover it.** Realtime voice rides a provider the owner has
  already connected (see [voice.md](../voice.md)); the call asks for no key of
  its own, as before.

### What stays

- The teammate does the work, in its own session and open chapter, with its
  own grants. The front has no tools of the teammate's reach and no other
  route to the room.
- Hanging up, holding or interrupting speech still does not cancel accepted
  teammate work.
- The existing pipeline (transcribe, teammate turn, synthesize) stays as the
  fallback when no realtime provider is connected, the budget is refused or
  the front fails mid-call. A fallback does not end the call.
- The tape and chat keep the person's words; the written version of the
  teammate's replies is what the chat shows.

### Guardrails

- The front **relays and clarifies**. It may greet, acknowledge, ask a
  question to sharpen the request, and report what the teammate has said. It
  never claims work is done, started or possible on its own word; only the
  teammate's events say that.
- The **client picks the spoken text**: what the person hears of the
  teammate's reply is chosen and stated by the core from the teammate's
  `spoken` version, and the front says it as given. The front does not
  paraphrase a report into a new claim.
- A front is enforced in the core outside the model: its seat, its budget
  reservation and its lease are the call's, and it is revocable by hanging up.
  A change to its reach follows [security.md](../security.md).

## Consequences

- `docs/voice-calls.md` replaces "No front" with the front's design; its wire
  ledger and `docs/voice.md` providers carry the front's entries.
- A call runs on the old pipeline unless Settings picks a front and the
  client asks for one.
