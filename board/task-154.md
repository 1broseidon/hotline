---
id: task-154
title: "P2-45a (D): Shared test plumbing: one bounded test Client in tests/common"
column: todo
position: 43
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:03.758Z"
updatedAt: "2026-10-17T22:47:18.398Z"
relatedFiles:
  - crates/hotline-core/tests/common/mod.rs
  - crates/hotline-core/tests/
  - crates/hotline-cli/tests/
---

## Description
Phase 2 group D, the first half of plan row 45 in docs/audits/0.40/README.md on release/0.40.0. Read tests-and-docs.md T4, T9, T14 and the Standards first. The `until(what, cond)` helper and the T8 sleep-then-assert sites are task-175, after this one.

One bounded test `Client` in `crates/hotline-core/tests/common/mod.rs`, then one file per commit for the seven copies. The CLI tests get a `crates/hotline-cli/tests/common` of their own. `tests/common` allows stay `#[allow(…, reason)]`, never `expect` (C5: each test binary uses a different subset).

End with `make precheck` and `make verify` (the hooks are on; never --no-verify). Follow the epic's rules.
