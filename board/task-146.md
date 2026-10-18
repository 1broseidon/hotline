---
id: task-146
title: "P2-37 (D): One `ANSWER` for the remote tests,"
column: in-progress
position: 35
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:59.590Z"
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/tests/
updatedAt: "2026-10-18T04:59:23.667Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:59:23.667Z"
    reworkCount: 0
dependsOn:
  - task-134
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:06.499Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:59:15.620Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 37 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One `ANSWER` for the remote tests,** at every wait for something that must arrive

- Source: tests-and-docs T2, task 2
- Files: `remote/tests.rs`, `remote/tests/*.rs`
- Collides with: 15, 25 (land first)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
