---
id: task-127
title: "P2-15 (C): One test-file naming style"
column: todo
position: 16
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:49.698Z"
relatedFiles:
  - crates/hotline-core/src/imagegen/chatgpt.rs
  - crates/hotline-core/src/imagegen/chatgpt_tests.rs
  - crates/hotline-core/src/driver/acp/auth_tests.rs
  - crates/hotline-core/src/remote/tests/
updatedAt: "2026-10-17T22:46:32.500Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 15 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One test-file naming style.** `chatgpt_tests.rs` → `chatgpt/tests.rs`, and the `#[path]` goes. `auth_tests.rs` moves under `acp/auth`. `remote/tests/*_tests.rs` → `<topic>.rs`

- Source: structure F9, task 2
- Files: `imagegen/chatgpt*`, `driver/acp/auth_tests.rs`, `remote/tests/*`
- Collides with: 25, 37, 38
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
