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
updatedAt: "2026-10-19T17:22:37.613Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T17:22:37.486Z"
  until: done
  tries: 0
---

## Description
ADR 0007 step 3. Move voice/speech* and voice/realtime* (about 7,700 production and 3,400 test lines) to crates/hotline-voice. Their reach outside voice: credentials (hotline-support), ProviderAuth and install_crypto_provider (hotline-room since epic-18), models::is_custom, three credential helpers in providers (ChatGPT's refreshed sign-in, OpenRouter's key, xAI's bearer) and the vault: move the helpers down with the engines or make them values the caller resolves and hands in, and say which in the ADR. sherpa-onnx, symphonia, webrtc, rtc and opus move with them and leave hotline-core's manifest if nothing else uses them. The Speech (five implementations) and Realtime (two) traits move as they are. Engine tests run with `cargo test -p hotline-voice`; the desk harness's voice tests (tests/voice, tests/local_speech, tests/processes) pass with their assertions unchanged; local_speech_check still runs (move the example with the engines if it only needs them, and fix the `cargo run` line in its doc and in docs/).

Lead's additions (2026-10-19):
- Branch is release/0.41.0. ADR 0007 currently ends with "step 3 waits on George" by its stopping rule. Record George's 2026-10-19 decision (from epic-19's description: split voice for maintainability, not build speed) in the ADR as what lets step 3 go ahead, then the step done.
- sherpa-onnx links a prebuilt archive fetched by scripts/fetch-speech-engine (make speech-engine); whatever points that link at hotline-core (env, build script, .cargo/config, precheck legs, CI) must follow the crate.
- Follow epic-18's pattern (task-208, task-209 in the log): the crate gets workspace lints, license and a crate doc saying what it owns and that it never depends on hotline-core or Tauri; each moved module renames its own crate:: paths; the core keeps `pub use` re-exports only while callers remain, and you list them in your notes; docs/standards.md's module map, docs/security.md citations and AGENTS.md follow the moves.
- In ADR 0007, list each moved file's old and new path: task-213 (the call-interface design) and epic-17 find the engines by that list. Take step 3's timings the way task-210 did.
- Do not touch the call files (call, duplex, front*, dispatcher, turn, synthesis, replies, record, input): task-213 designs their move.
