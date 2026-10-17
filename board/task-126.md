---
id: task-126
title: "P2-14 (C): `foo.rs` + `foo/`"
column: todo
position: 15
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:49.183Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 14 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`foo.rs` + `foo/`.** `git mv` all 16 `mod.rs` files; `git diff -M` shows 100% similarity

- Source: structure F9, task 1
- Files: the 16 `mod.rs`
- Collides with: everything under `src/`: land with no other Rust branch open
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
