---
id: task-134
title: "P2-25 (C): `remote/tests.rs`"
column: todo
position: 23
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:53.318Z"
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/tests/
updatedAt: "2026-10-17T22:46:55.761Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 25 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`remote/tests.rs`** topic split. Pulled forward from structure's step 18 so that the network fixes (37, 38) can follow

- Source: structure F6, task 18 (part)
- Files: `remote/tests.rs`
- Collides with: 15, 37, 38
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
