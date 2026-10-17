---
id: task-167
title: "P2-52 (F): The `[workspace.lints]` table in one change"
column: todo
position: 56
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:10.558Z"
---

## Description
Phase 2 group F (Gates), plan row 52 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The `[workspace.lints]` table in one change.** It holds every lint that 39–42 made clean, the zero-hit denies, and `allow_attributes_without_reason`. Add `rust-version`, and `[lints] workspace = true` in all three crates. `-D warnings` stays in the Makefile

- Source: lints, tasks 1 and 4 (C4, C5)
- Files: `Cargo.toml`, three crate manifests
- Collides with: 13
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
