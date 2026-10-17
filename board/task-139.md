---
id: task-139
title: "P2-30 (C): `wire/commands.rs`:"
column: todo
position: 28
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:55.925Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 30 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/commands.rs`:** `run` only dispatches, and the arms go to `commands/{persona,settings,models}.rs`

- Source: structure F7, task 16
- Files: `wire/commands.rs`
- Collides with: 41 (`mobile_persona_create`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
