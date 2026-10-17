---
id: task-140
title: "P2-31 (C): The six other over-budget files"
column: todo
position: 29
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:56.439Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 31 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The six other over-budget files**, one commit each: `session/peers.rs`, `tools/workspace.rs`, `session/sides.rs`, `computer/mod.rs`, `driver/rig.rs`, `driver/rig/turn.rs` (its tests, plus the `run_inner` phases with the retry rule kept in one place)

- Source: structure F8, task 17
- Files: those files
- Collides with: 41 (`rig/turn.rs:689`), 42 (rig `Failure` sites), 47 (`turn.rs` sleeps)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
