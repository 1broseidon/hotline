---
id: task-149
title: "P2-40 (D): By-hand lints and reasoned invariants"
column: todo
position: 38
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:01.159Z"
relatedFiles:
  - crates/hotline-core/src/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/
updatedAt: "2026-10-17T22:46:59.581Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 40 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**By-hand lints and reasoned invariants.** The "[2]" lints and the one- to five-hit pedantic picks, reading `suspicious_operation_groupings`, `missing_fields_in_debug` and `unchecked_time_subtraction` first. Then replace the 26 non-lock `unwrap`s with reasoned `expect`s, and give the empty `unreachable!`s the narrowing that justifies them

- Source: lints, task 3; errors F5, task 6
- Files: the hit sites; the clusters in the errors.md "Leave alone" table
- Collides with: 39
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
