---
id: task-169
title: "P2-54 (F): The window's gates"
column: todo
position: 58
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:11.612Z"
relatedFiles:
  - ui/knip.json
  - ui/package.json
  - ui/bun.lock
  - Makefile
updatedAt: "2026-10-18T03:33:54.818Z"
contract:
  status: ready
  validation:
    commands:
      - test -f ui/knip.json
      - grep -q knip Makefile ui/package.json
      - make check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-168
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:30.025Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:33:54.818Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 54 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The window's gates.** Pinned knip with both HTML entries, tests, generated types, and the mobile-copy exports and `computerViewer.css` exempt. A contract preflight that fails in one sentence when `contract.ts` is partial

- Source: dead-code F4, task 16; window F10, task 5
- Files: `ui/knip.json`, `ui/package.json`, `Makefile`
- Collides with: 10
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
