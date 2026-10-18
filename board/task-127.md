---
id: task-127
title: "P2-15 (C): One test-file naming style"
column: review
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
updatedAt: "2026-10-18T02:39:56.489Z"
contract:
  status: delivered
  validation:
    commands:
      - "! git ls-files crates | grep -E '_tests\\.rs$'"
      - "! git grep -n '#\\[path' -- crates"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:24:00.165Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T02:39:56.489Z"
    duration: 956
dependsOn:
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:45.167Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:23:33.590Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 15 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One test-file naming style.** `chatgpt_tests.rs` → `chatgpt/tests.rs`, and the `#[path]` goes. `auth_tests.rs` moves under `acp/auth`. `remote/tests/*_tests.rs` → `<topic>.rs`

- Source: structure F9, task 2
- Files: `imagegen/chatgpt*`, `driver/acp/auth_tests.rs`, `remote/tests/*`
- Collides with: 25, 37, 38
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T02:39:56.014Z: [claude] Done on dispatch/task-127, two commits. 1: imagegen/chatgpt_tests.rs -> chatgpt/tests.rs (100%, #[path] gone); driver/acp/auth_tests.rs -> acp/auth/tests.rs, declared in auth.rs, imports now crate::driver::acp::{tests::…, *}, and the re-exec filter in it is driver::acp::auth::tests::…; remote/tests/{admission,link,relay,viewer_file,viewer_token}_tests.rs -> <topic>.rs (3 at 100%). The child module remote::tests::relay now shadows remote::relay for the sibling topics, so the two relay::paired calls (link.rs, relay.rs) spell crate::remote::relay::paired — rows 25/37/38 should do the same when they need remote::relay from inside remote/tests. docs/security.md and docs/serve.md cite the new paths/filters (cargo test remote::tests::admission). 2: outside the row's file list but caught by the contract's #[path] check: hotline-app/tests/shell_path.rs no longer #[path]-includes src/shell_path.rs; lib.rs makes `pub mod shell_path` (cfg unix) and the test uses hotline_app_lib::shell_path. Inline `mod *_tests {}` blocks (releases.rs, imagegen.rs, remote/bridge.rs, client.rs, relay.rs, room.rs, schedule.rs, speech.rs, openai_shape.rs, commands.rs) were left: they are in-file modules, not files. Validation passes; make check and make precheck (7/7) pass. Nothing left unwired.
