---
id: task-129
title: "P2-17 (C): Split `contract.rs`"
column: todo
position: 18
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:50.735Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 17 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Split `contract.rs`** into the nine submodules, re-exported. `contract.ts` stays byte-identical

- Source: structure F3, task 4
- Files: `contract.rs` → `contract/*.rs`
- Collides with: 16
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
