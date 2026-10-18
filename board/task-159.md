---
id: task-159
title: "P2-50 (D): a11y pass:"
column: todo
position: 48
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:06.356Z"
relatedFiles:
  - ui/src/components/
  - ui/src/App.tsx
updatedAt: "2026-10-18T03:12:55.940Z"
contract:
  status: ready
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-156
  - task-158
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:21.041Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:12:55.940Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 50 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**a11y pass:** the 9 `noAutofocus`, 8 `useSemanticElements` and 8 `noStaticElementInteractions` hits, as separate small UI changes

- Source: window F6, task 14
- Files: the hit sites
- Collides with: 33, 48
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
