---
id: task-152
title: "P2-43 (D): Paused clock"
column: todo
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
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - grep -q 'test-util' crates/hotline-core/Cargo.toml
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-146
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 43 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Paused clock** for the 30 s viewer-token and 10 s xAI deadline tests (`tokio` `test-util` as a dev feature). The core lib binary should drop below 10 s

- Source: tests-and-docs T5, task 4
- Files: `Cargo.toml:16`, `crates/hotline-core/Cargo.toml:14`, `remote/tests/viewer_token.rs`, `voice/speech/xai/tests.rs:819`
- Collides with: 15
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
