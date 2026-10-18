---
id: task-137
title: "P2-28 (C): `voice/mod.rs`"
column: review
position: 26
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.875Z"
relatedFiles:
  - crates/hotline-core/src/voice.rs
  - crates/hotline-core/src/voice/call.rs
  - crates/hotline-core/src/voice/input.rs
  - crates/hotline-core/src/voice/turn.rs
  - crates/hotline-core/src/voice/synthesis.rs
updatedAt: "2026-10-18T04:35:56.010Z"
contract:
  status: delivered
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/voice.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:14:43.476Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T04:35:56.010Z"
    duration: 1273
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:56.374Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:14:28.326Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 28 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`voice/mod.rs`** → `call.rs`, `input.rs`, `turn.rs`, `synthesis.rs`

- Source: structure F4, task 14
- Files: `voice.rs`, `voice/*`
- Collides with: 41 (`start_call`, `speak_sentence`), 40
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:36:00.366Z: [claude] Done on dispatch/task-137 (86bdc657), move-only: voice.rs is now 951 lines (types, new/status/speech models, public accessors). voice/call.rs has start*, thinking_again, bind_connection, subscribe, change, end, finish, hold, interrupt. voice/input.rs has utterance, text, relay, queue_*, resolve_speech, speech_for, connected, login_connected, audio, validate_audio, mp4_duration. voice/turn.rs has emit through pay. voice/synthesis.rs has synthesize, say*, speaking, speak_sentence, synthesize_stream, clip, calling, begin_answer, speech_ready, goodbye, own_voice, sentences, take_sentences, spoken_lengths. Methods that were private are now pub(super); the bodies are unchanged apart from rustfmt rewrapping, and the include_bytes paths became assets/*. Callers in dispatcher/duplex/spoken now call super::synthesis::*. tests.rs imports what it used to get through the glob (Context, VoiceCallMode, RoomHandle, the moved free fns). For row 41: start_call is in voice/call.rs and speak_sentence is in voice/synthesis.rs. make check and make precheck pass, and contract.ts is unchanged. processes.rs process_chip_a_process_is_on_the_session_until_the_person_stops_it timed out once under a full make check; it passed when run alone and on the rerun. Nothing is left unwired.
