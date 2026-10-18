---
id: task-149
title: "P2-40 (D): By-hand lints and reasoned invariants"
column: in-progress
position: 0
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
updatedAt: "2026-10-18T08:43:43.190Z"
contract:
  status: in_progress
  validation:
    commands:
      - "! git grep -nE 'unreachable!\\(\\)' -- 'crates/*/src'"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T08:43:43.190Z"
    reworkCount: 1
dependsOn:
  - task-148
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T08:43:31.268Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T08:43:37.552Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 40 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**By-hand lints and reasoned invariants.** The "[2]" lints and the one- to five-hit pedantic picks, reading `suspicious_operation_groupings`, `missing_fields_in_debug` and `unchecked_time_subtraction` first. Then replace the 26 non-lock `unwrap`s with reasoned `expect`s, and give the empty `unreachable!`s the narrowing that justifies them

- Source: lints, task 3; errors F5, task 6
- Files: the hit sites; the clusters in the errors.md "Leave alone" table
- Collides with: 39
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

Related Files:
  - crates/hotline-core/src/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/

Settle note (Pork Chop, 2026-10-18 03:45): the run stopped as 'done' with crates/hotline-core/src/log.rs half-edited and uncommitted (a nonminimal_bool rewrite to is_none_or, not cargo-fmt'd) and no done note. Finish the row: the remaining [2] lints and the reasoned expect/unreachable work. Commit each through the hooks, run make precheck, then write the done note with what was covered and what was left on purpose.

## Log
- 2026-10-18T08:39:54.251Z: Not landed: 1 file is not committed in the worktree: send it back, or commit in your editor
