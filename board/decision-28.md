---
id: decision-28
title: Legacy streams the teammate's own opening, sends one soft cue before audio, and rotates neutral…
type: decision
status: current
claimKey: voice.legacy-feedback
kind: decision
recheck: Verify audibility and cadence in physical Linux desk and phone QA; generated clips are not proof of playback.
supersedes: decision-17
createdAt: "2026-10-21T20:00:39.211Z"
---

## Description
Legacy streams the teammate's own opening, sends one soft cue before audio, and rotates neutral progress lines during quiet work.
Legacy speaks the teammate's own opening as it arrives, without rewriting it. After 1.8 seconds without an audio clip, the core sends one wordless cue on the ordinary clip wire; a Said text event awaiting TTS does not suppress it. While a teammate turn remains active, twenty seconds without audio queues a short core-owned first-person line on the normal speech queue. Neutral lines rotate without consecutive repeats and assert no result, imminent completion or hand-off. The queue rechecks the turn, recent audio, hold, caller speech and cancellation before saying it. Completion, interruption and superseding input stop stale feedback. Clients add no repeating chime.
