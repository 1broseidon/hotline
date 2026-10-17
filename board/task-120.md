---
id: task-120
title: "P2-8 (B): Rust dead code"
column: todo
position: 9
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:46.077Z"
relatedFiles:
  - crates/hotline-core/src/paths.rs
  - crates/hotline-core/src/process_windows.rs
  - crates/hotline-app/src/desks.rs
updatedAt: "2026-10-17T22:46:30.605Z"
---

## Description
Phase 2 group B (Delete dead code), plan row 8 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Rust dead code.** Delete `paths::resolve_command` and its docs (keep `resolve_command_in`). Drop the redundant `#[allow(dead_code)]` on `Job`'s field, which `terminate` reads. Check, and delete, the never-read collection at `desks.rs:660`

- Source: dead-code F5, F6, tasks 8 and 9; lints `collection_is_never_read`, `dead_code`
- Files: `crates/hotline-core/src/paths.rs:183`, `process_windows.rs:38`, `crates/hotline-app/src/desks.rs:660`
- Collides with: none
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
