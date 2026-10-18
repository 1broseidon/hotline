---
id: task-152
title: "P2-43 (D): Paused clock"
column: in-progress
position: 41
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:02.722Z"
relatedFiles:
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - Cargo.lock
  - crates/hotline-core/src/remote/tests/viewer_token.rs
  - crates/hotline-core/src/voice/speech/xai/tests.rs
updatedAt: "2026-10-18T05:22:15.658Z"
contract:
  status: in_progress
  validation:
    commands:
      - grep -q 'test-util' crates/hotline-core/Cargo.toml
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T05:22:15.658Z"
    reworkCount: 0
dependsOn:
  - task-146
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:13.251Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:21:29.942Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 43 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Paused clock** for the 30 s viewer-token and 10 s xAI deadline tests (`tokio` `test-util` as a dev feature). The core lib binary should drop below 10 s

- Source: tests-and-docs T5, task 4
- Files: `Cargo.toml:16`, `crates/hotline-core/Cargo.toml:14`, `remote/tests/viewer_token.rs`, `voice/speech/xai/tests.rs:819`
- Collides with: 15
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:37:21.374Z: [claude] Done on dispatch/task-152 (6c22df76). tokio test-util is now a hotline-core dev-dependency. The viewer-token expiry reads tokio::time::Instant (bridge.rs) so the paused clock moves it. Neither test uses start_paused, because a clock paused from the start let time jump during real loopback reads: the xAI test failed 2 runs in 5 with connect finishing at virtual 10 s. Instead, viewer_token calls pause() just before its 30 s sleep and resume() right after it. The xAI deadline test pauses only after the server gets a pong from the client's readiness wait, so the connection is up when it pauses (20/20 runs pass). The core lib binary went from 36 s to about 16 s, not below 10 s. Its floor is the 11 s relay revocation test (real kernel buffer, kept on purpose by the audit), then pause 8 s, admission eviction 6.3 s, rig 'every line' 6.2 s, and the 5.2-5.5 s pre-auth/recovery expiry tests. Those are the next candidates if someone wants the binary faster. make check and make precheck pass.
