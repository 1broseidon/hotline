---
id: decision-17
title: Legacy streams the teammate's own opening, sends one soft cue before audio, and speaks bounded…
type: decision
status: current
claimKey: voice.legacy-feedback
kind: decision
recheck: Verify audibility and cadence in physical Linux desk and phone QA; generated clips are not proof of playback.
supersedes: decision-13
createdAt: "2026-10-21T08:45:31.035Z"
---

## Description
Legacy streams the teammate's own opening, sends one soft cue before audio, and speaks bounded progress during quiet work.
Legacy speaks the teammate's own opening as it arrives, without rewriting it. After 1.8 seconds without an audio clip, the core sends one wordless cue on the ordinary clip wire; a Said text event awaiting TTS does not suppress it. While a teammate turn remains active, twenty seconds without audio queues the core-owned line “I'm still working on that.” on the normal speech queue. It asserts no tool result. The queue rechecks the turn, recent audio, hold, caller speech and cancellation before saying it. Completion, interruption and superseding input stop stale feedback. Clients add no repeating chime.
