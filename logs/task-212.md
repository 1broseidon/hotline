---
id: task-212
title: "hotline-voice: the speech and realtime engines move first"
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
  status: delivered
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
    pickedUpAt: "2026-10-19T17:22:50.262Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T18:21:03.399Z"
    duration: 3493
updatedAt: "2026-10-19T18:23:10.149Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T17:22:37.486Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T17:22:38.944Z"
    ended: "2026-10-19T18:21:11.090Z"
    outcome: review
completedAt: "2026-10-19T18:23:10.149Z"
---

## Description
ADR 0007 step 3. Move voice/speech* and voice/realtime* (about 7,700 production and 3,400 test lines) to crates/hotline-voice. Their reach outside voice: credentials (hotline-support), ProviderAuth and install_crypto_provider (hotline-room since epic-18), models::is_custom, three credential helpers in providers (ChatGPT's refreshed sign-in, OpenRouter's key, xAI's bearer) and the vault: move the helpers down with the engines or make them values the caller resolves and hands in, and say which in the ADR. sherpa-onnx, symphonia, webrtc, rtc and opus move with them and leave hotline-core's manifest if nothing else uses them. The Speech (five implementations) and Realtime (two) traits move as they are. Engine tests run with `cargo test -p hotline-voice`; the desk harness's voice tests (tests/voice, tests/local_speech, tests/processes) pass with their assertions unchanged; local_speech_check still runs (move the example with the engines if it only needs them, and fix the `cargo run` line in its doc and in docs/).

Lead's additions (2026-10-19):
- Branch is release/0.41.0. ADR 0007 currently ends with "step 3 waits on George" by its stopping rule. Record George's 2026-10-19 decision (from epic-19's description: split voice for maintainability, not build speed) in the ADR as what lets step 3 go ahead, then the step done.
- sherpa-onnx links a prebuilt archive fetched by scripts/fetch-speech-engine (make speech-engine); whatever points that link at hotline-core (env, build script, .cargo/config, precheck legs, CI) must follow the crate.
- Follow epic-18's pattern (task-208, task-209 in the log): the crate gets workspace lints, license and a crate doc saying what it owns and that it never depends on hotline-core or Tauri; each moved module renames its own crate:: paths; the core keeps `pub use` re-exports only while callers remain, and you list them in your notes; docs/standards.md's module map, docs/security.md citations and AGENTS.md follow the moves.
- In ADR 0007, list each moved file's old and new path: task-213 (the call-interface design) and epic-17 find the engines by that list. Take step 3's timings the way task-210 did.
- Do not touch the call files (call, duplex, front*, dispatcher, turn, synthesis, replies, record, input): task-213 designs their move.

## Log
- 2026-10-19T18:21:02.933Z: [claude] Done on dispatch/task-212 (dd4923db code, 7025f7f6 ADR+docs). crates/hotline-voice (on contract, support, room; never core or Tauri) holds speech.rs+speech/ and realtime.rs+realtime/ as they were (ADR 0007 "Step 3, done" has the old→new table for task-213/epic-17). Seam: no new trait; Speech and Realtime moved as they were. The vault reaches the engines as speech::Connected { root, providers: Vec<(id, ProviderAuth)> in connect order, sign_ins }, read by the core's new voice/connected.rs. Helpers: none moved down. OpenRouter's key is a value (core turns that sign-in into ProviderAuth::ApiKey). xAI's bearer and ChatGPT's refresh are fn pointers in speech::SignIns (xai::Bearer, realtime::live::SignIn), pointed at providers::xai::bearer / chatgpt::refreshed_auth. They stay in the core because the xAI TokenStore is one process-wide refresh lock shared with the Rig clients, and ChatGPT's refresh is Rig's. Also moved: front::SEND_TO_TEAMMATE → hotline_voice::realtime (front.rs re-exports it, a one-line change, the only call file touched) and models::is_custom → hotline_room::provider_auth. Re-exports the core keeps while callers remain: crate::voice::speech (glob of hotline_voice::speech, with the core's five &Vault readers resolve/resolve_output/resolve_front/listens_for/options in place of the engines' namesakes; ~15 src files plus tests/voice, tests/voice/front, tests/local_speech, examples/speech_check), crate::voice::realtime (6 src files + 2 harness files), crate::voice::front::SEND_TO_TEAMMATE (3), crate::models::is_custom (2). webrtc/rtc/opus/sherpa-onnx/symphonia left core's manifest; SHERPA_ONNX_ARCHIVE_DIR is workspace-relative, so nothing else needed changing. Tests: 149 in the crate (146 run + 3 ignored live probes); the 3 tests that use a real Vault are in voice/connected.rs. Engine xAI tests use a test login (xai::test_bearer/sign_in_for_test/test_login), assertions unchanged. The ignored Grok/GPT-Live probes now use the stored login unrefreshed. The AAC test reads hotline-core/tests/fixtures/voice/ask-mack.m4a. Harness tests unchanged; local_speech_check stays in core (needs Desk) and ran OK. Module map: strongly connected set 17→14 (capabilities, import, desk left it). Timings (419bdac5 vs dd4923db, own worktrees, interleaved): lib 28.1→24.6 s, front end 17.7→15.3 s, cold build 29.8→26.3 s, cold test build 59.9→54.1 s, touch floors 3.0/8.0→2.7/7.7 s, engines alone 1.1 s; an engine edit built by the core went 3.1→3.4 s. For task-213: the calls (input.rs, duplex.rs) still call speech::resolve*(&self.vault, …) through the core's readers; when they move, they take a Connected (or the room hands one in) and voice/connected.rs is what's left in the core. Docs: standards.md, development.md, AGENTS.md, security.md (2 citations), voice.md, THIRD-PARTY-NOTICES, 0.40 audits' full-path citations (README notes the exception). make check, make precheck (8 legs) and every contract validation line pass; contract.ts identical to 419bdac5.
- 2026-10-19T18:23:09.768Z: Landed on release/0.41.0
