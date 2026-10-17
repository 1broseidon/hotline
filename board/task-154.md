---
id: task-154
title: "P2-45 (D): Shared test plumbing"
column: todo
position: 43
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:03.758Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 45 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Shared test plumbing.** One bounded test `Client` in `tests/common/mod.rs`, then one file per commit for the seven copies. The CLI tests get a `tests/common`. Add one `until(what, cond)` with one deadline, then replace the T8 sleep-then-assert sites module by module

- Source: tests-and-docs T4, T8, T9, T14, tasks 7, 8, 18
- Files: `crates/hotline-core/tests/*.rs`, `tests/common/{mod,stalled}.rs`, `crates/hotline-cli/tests/*`, and the T8/T9 sites
- Collides with: 23, 24, 32 (land first)
- Size: L

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
