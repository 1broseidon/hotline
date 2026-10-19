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
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-voice/Cargo.toml
      description: the voice engines crate (speech and realtime)
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: George's 2026-10-19 decision, step 3 done with each moved file's old and new path, the credential-helper choice, and step 3's timings
  validation:
    commands:
      - test -f crates/hotline-voice/Cargo.toml
      - "! test -e crates/hotline-core/src/voice/speech.rs"
      - "! test -e crates/hotline-core/src/voice/speech"
      - "! test -e crates/hotline-core/src/voice/realtime.rs"
      - "! test -e crates/hotline-core/src/voice/realtime"
      - "! grep -qE '^hotline-core[[:space:]]*=' crates/hotline-voice/Cargo.toml"
      - "! grep -qE '^tauri' crates/hotline-voice/Cargo.toml"
      - grep -q 'workspace = true' crates/hotline-voice/Cargo.toml
      - cargo test -p hotline-voice
      - cargo test -p hotline-core --test voice --test local_speech --test processes
      - cargo build --workspace --examples
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - Moves only, no behaviour change; the Speech and Realtime traits move as they are, no new trait
    - hotline-voice never depends on hotline-core or Tauri
    - ui/src/generated/contract.ts byte-identical after an unfiltered cargo test
    - Harness tests keep their assertions; only their import paths may follow the move
    - Run cargo test, make check and make precheck in the foreground and to completion before ending the turn
  metrics:
    readyAt: "2026-10-19T17:22:10.798Z"
updatedAt: "2026-10-19T17:22:10.798Z"
---

## Description
ADR 0007 step 3. Move voice/speech* and voice/realtime* (about 7,700 production and 3,400 test lines) to crates/hotline-voice. Their reach outside voice: credentials (hotline-support), ProviderAuth and install_crypto_provider (hotline-room since epic-18), models::is_custom, three credential helpers in providers (ChatGPT's refreshed sign-in, OpenRouter's key, xAI's bearer) and the vault: move the helpers down with the engines or make them values the caller resolves and hands in, and say which in the ADR. sherpa-onnx, symphonia, webrtc, rtc and opus move with them and leave hotline-core's manifest if nothing else uses them. The Speech (five implementations) and Realtime (two) traits move as they are. Engine tests run with `cargo test -p hotline-voice`; the desk harness's voice tests pass unchanged; local_speech_check still runs.
