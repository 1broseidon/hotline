---
id: task-153
title: "P2-44 (D): Honest tests"
column: todo
position: 42
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:03.238Z"
relatedFiles:
  - crates/hotline-core/tests/desk.rs
  - crates/hotline-core/tests/local_speech.rs
  - crates/hotline-core/src/tools/shell.rs
  - crates/hotline-core/tests/processes.rs
  - crates/hotline-core/tests/voice.rs
  - crates/hotline-core/src/tools/shell/macos.rs
updatedAt: "2026-10-17T22:47:18.127Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 44 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Honest tests.** Opt-in tests use `#[ignore = "needs …"]` instead of returning early. Elapsed-time ceilings become orderings or wide gaps. A missing `python3` or `perl` fails by name

- Source: tests-and-docs T6, T7, T10, tasks 5, 6, 9
- Files: `tests/desk.rs:926,1041,1195`, `tests/local_speech.rs:428`, `tools/shell.rs:585-595,968`, the six T6 sites, `tests/processes.rs:28,385`, `tests/voice.rs:2826`, `tools/shell/macos.rs:476`
- Collides with: 32
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
