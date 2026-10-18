---
id: task-168
title: "P2-53 (F): Biome for lint only"
column: todo
position: 57
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:11.082Z"
relatedFiles:
  - ui/biome.json
  - ui/package.json
  - ui/bun.lock
  - Makefile
updatedAt: "2026-10-18T03:26:03.467Z"
contract:
  status: ready
  validation:
    commands:
      - test -f ui/biome.json
      - grep -q biome Makefile ui/package.json
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-159
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:28.895Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:26:03.467Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 53 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Biome for lint only.** `biome.json` with the window.md rule set, formatter off, a11y at warn until 50 lands, and a `lint` step in `make ui-check`

- Source: window F6, task 7
- Files: `ui/biome.json`, `ui/package.json`, `Makefile`
- Collides with: 48
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
