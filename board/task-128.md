---
id: task-128
title: "P2-16 (C): `contract` becomes a leaf"
column: todo
position: 17
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:50.224Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 16 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`contract` becomes a leaf.** Move the types it borrows into it, with no re-export left behind. `contract.ts` stays byte-identical

- Source: structure F3, F10, task 3
- Files: `contract.rs`, `thread`, `remote`, `linked`, `imagegen`, `spending`, and their callers
- Collides with: 17
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
