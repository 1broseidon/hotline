---
id: task-160
title: "P2-51 (D): Audit lenses, with no table entry"
column: todo
position: 49
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
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-151
  - task-145
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 51 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Audit lenses, with no table entry.** Read the 23 `significant_drop_in_scrutinee` and 114 `large_futures` hits, and the `exit` and `mem_forget` sites. Adopt a lint only if a bug class turns up

- Source: lints, task 7
- Files: read-only, then fixes as found
- Collides with: 36
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
