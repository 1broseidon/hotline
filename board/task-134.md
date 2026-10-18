---
id: task-134
title: "P2-25 (C): `remote/tests.rs`"
column: in-progress
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
updatedAt: "2026-10-18T04:14:16.191Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/remote/tests.rs) -le 2000
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:00:40.779Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:53.027Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:11:51.377Z"
    ended: "2026-10-18T04:14:16.191Z"
    outcome: review
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 25 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`remote/tests.rs`** topic split. Pulled forward from structure's step 18 so that the network fixes (37, 38) can follow

- Source: structure F6, task 18 (part)
- Files: `remote/tests.rs`
- Collides with: 15, 37, 38
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:13:43.638Z: [claude] Split committed (151020ca): remote/tests.rs 3406→491 lines (harness + shared helpers), 10 new topic files beside the existing 6; 77 tests before and after, same fn names, move-only apart from super::→crate::remote:: paths; security.md test index repointed. Running make check / precheck.
