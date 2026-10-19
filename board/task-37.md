---
id: task-37
title: Apple on-device transcription fails silently when Speech Recognition is refused
column: review
position: 4
tags:
  - bug
  - bro-272
createdAt: "2026-10-11T21:06:00.652Z"
description: |-
  Split out of BRO-271 (which shipped the models and hotwords in 0.36.0).

  **Seen 2026-10-09** on George's Mac. After a self-signed local build with the same bundle id (`dev.hotline.app`) was run, the official app's Speech Recognition grant no longer matched its signature. The mic worked and the level meter moved, but "On this Mac" transcription produced nothing: no error, no text. Switching to the desk's Parakeet worked.

  **Fix:**

  * When on-device recognition is refused, unavailable or produces an error, say so on the call card or in dictation, e.g. "Hotline can't use Speech Recognition. Allow it in System Settings › Privacy & Security."
  * Fall back to an installed desk model for that call or dictation.
  * Never leave an open mic that silently produces nothing.

  **Dev note:** never run a self-signed copy with the release bundle id on a machine with the real app installed. It confuses TCC (and the keychain). For local testing use the dev build, or a signed beta.

  Migrated from Linear BRO-272: https://linear.app/1broseidon/issue/BRO-272/apple-on-device-transcription-fails-silently-when-speech-recognition (2026-10-11).

  ## Source audit 2026-10-11

  Corrected the earlier review: call.ts:744 handles native error events and ended(error), and dictation.ts:259 does too. Those reporting paths already exist. What remains is reproducing the signature/TCC case where the mic moves without any transcript or error, showing an actionable refusal/unavailable state, and offering/using an installed desk model without silent paid fallback. Do not treat ordinary silence as a permission failure. An active call's input mode is fixed (voice/mod.rs:659); any fallback from device text to desk audio must explicitly replace/restart the call rather than change its mode in place. Verify native refused/unavailable, emitted runtime errors, silent recognition failure and installed-model recovery for both call and dictation.

  0.40 triage (2026-10-18): Remaining: the refused and failed messages exist (ui/src/voice/call.ts, dictation.ts). Detect a microphone that runs, with the level meter moving, but never yields text (the signature or permission mismatch case), say so in a few words, and offer an installed desk model.
updatedAt: "2026-10-19T00:49:29.253Z"
priority: high
parentId: epic-14
relatedFiles:
  - ui/src/voice/call.ts
  - ui/src/voice/dictation.ts
contract:
  status: delivered
  deliverables:
    - type: test
      path: ui/src/voice
      description: a running mic with a moving level and no transcript is reported in a few words and offers an installed desk model; ordinary silence is not
  validation:
    commands:
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:44:12.367Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:49:11.180Z"
    duration: 299
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:43:57.816Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:44:01.478Z"
    ended: "2026-10-19T00:49:18.669Z"
    outcome: review
assignee: claude
---

## Log
- 2026-10-19T00:49:14.700Z: [claude] Done on branch dispatch/task-37 (6513ea28), make ui-check green. WordlessVoice (ui/src/voice/transcription.ts) trips after 3 s of voice the TurnDetector found, across turns and engine sessions, while the Mac's engine has never answered a word; silence and anything after a first word never trip it. Call ends with trouble UNHEARD ("No words heard" on the card); dictation stops with the same sentence via macEngine. "Use desk model" (call card + composer) sets hearOnThisMac off and redials/redictates, offered only when voice.status {} reports stt.providerId "local" (hearsWithDeskModel/useDeskModelOffer in desk.ts), so no paid fallback. Not done: no real-TCC repro (needs a signature-mismatched build); the composer offer has no render test, only the call offer is unit-tested. Documented in docs/voice.md.
- 2026-10-19T00:49:29.253Z: Landed on release/0.40.0
