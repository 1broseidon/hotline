---
id: task-37
title: Apple on-device transcription fails silently when Speech Recognition is refused
column: todo
position: 39
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

  ## Status (2026-10-11 review)

  Only a start failure is reported today (ui/src/voice/call.ts ~115 NO_TRANSCRIPTION); an open mic that produces nothing and fallback to a desk model are unhandled.
updatedAt: "2026-10-11T21:06:00.939Z"
---
