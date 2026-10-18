---
id: task-160
title: "P2-51 (D): Audit lenses, with no table entry"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:06.879Z"
relatedFiles:
  - crates/hotline-core/src/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/
updatedAt: "2026-10-18T10:49:31.515Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T10:04:10.837Z"
    reworkCount: 0
dependsOn:
  - task-151
  - task-145
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:22.153Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T10:04:02.211Z"
    ended: "2026-10-18T10:49:31.515Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T10:49:31.126Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 51 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Audit lenses, with no table entry.** Read the 23 `significant_drop_in_scrutinee` and 114 `large_futures` hits, and the `exit` and `mem_forget` sites. Adopt a lint only if a bug class turns up

- Source: lints, task 7
- Files: read-only, then fixes as found
- Collides with: 36
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T10:49:30.752Z: Landed on release/0.40.0
