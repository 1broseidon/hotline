---
id: task-212
title: "hotline-voice: the speech and realtime engines move first"
column: todo
position: 9
priority: high
tags:
  - crate-split
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/speech.rs
  - crates/hotline-core/src/voice/speech
  - crates/hotline-core/src/voice/realtime.rs
  - crates/hotline-core/src/voice/realtime
parentId: epic-19
createdAt: "2026-10-19T17:20:18.735Z"
---

## Description
ADR 0007 step 3. Move voice/speech* and voice/realtime* (about 7,700 production and 3,400 test lines) to crates/hotline-voice. Their reach outside voice: credentials (hotline-support), ProviderAuth and install_crypto_provider (hotline-room since epic-18), models::is_custom, three credential helpers in providers (ChatGPT's refreshed sign-in, OpenRouter's key, xAI's bearer) and the vault: move the helpers down with the engines or make them values the caller resolves and hands in, and say which in the ADR. sherpa-onnx, symphonia, webrtc, rtc and opus move with them and leave hotline-core's manifest if nothing else uses them. The Speech (five implementations) and Realtime (two) traits move as they are. Engine tests run with `cargo test -p hotline-voice`; the desk harness's voice tests pass unchanged; local_speech_check still runs.
