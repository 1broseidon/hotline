---
id: task-132
title: "P2-23 (C): `session/tests.rs`"
column: todo
position: 21
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:52.292Z"
relatedFiles:
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/session/tests/
updatedAt: "2026-10-17T22:46:55.214Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 23 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`session/tests.rs`** → fixtures plus `session/tests/*.rs`, 131 tests before and after

- Source: structure F6, task 10
- Files: `session/tests.rs`
- Collides with: 1 (new test: land 1 first), 45
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
