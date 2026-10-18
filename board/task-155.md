---
id: task-155
title: "P2-46 (D): Search failures get a kind"
column: todo
position: 44
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:04.277Z"
relatedFiles:
  - crates/hotline-core/src/websearch.rs
  - crates/hotline-core/src/websearch/
updatedAt: "2026-10-18T02:40:12.004Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:16.610Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:40:12.004Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 46 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Search failures get a kind.** A local enum with status and retry-after picks the cooldown; the rendered text stays at the edge

- Source: errors F6, task 7
- Files: `websearch/mod.rs:108,193,687` and the adapters
- Collides with: 39
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
