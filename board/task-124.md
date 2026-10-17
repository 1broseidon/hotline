---
id: task-124
title: "P2-12 (B): Deduplicate `@tauri-apps/api`"
column: todo
position: 13
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:48.155Z"
---

## Description
Phase 2 group B (Delete dead code), plan row 12 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Deduplicate `@tauri-apps/api`** onto the tested direct pin

- Source: dead-code F9, task 13
- Files: `ui/bun.lock:687`
- Collides with: 6, 7
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
