---
id: task-137
title: "P2-28 (C): `voice/mod.rs`"
column: todo
position: 26
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.875Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 28 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`voice/mod.rs`** → `call.rs`, `input.rs`, `turn.rs`, `synthesis.rs`

- Source: structure F4, task 14
- Files: `voice.rs`, `voice/*`
- Collides with: 41 (`start_call`, `speak_sentence`), 40
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
