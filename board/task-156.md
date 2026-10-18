---
id: task-156
title: "P2-47 (D): Updater and settings errors use Refusal details"
column: in-progress
position: 45
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:04.793Z"
relatedFiles:
  - crates/hotline-app/src/updater.rs
  - ui/src/components/UpdatesSection.tsx
  - ui/src/components/ConnectProvider.tsx
  - ui/src/components/Skills.tsx
  - ui/src/components/Secrets.tsx
  - ui/src/components/ErrorCard.tsx
  - ui/src/wire.ts
updatedAt: "2026-10-18T02:40:55.650Z"
contract:
  status: in_progress
  validation:
    commands:
      - make ui-check
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:40:55.650Z"
    reworkCount: 0
dependsOn:
  - task-126
  - task-157
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:17.719Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:40:38.014Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 47 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Updater and settings errors use Refusal details.** First "Update check failed" with a disclosure, then provider connection, skills, secrets, and the ErrorCard retry. Labels stay within decision-2

- Source: errors F7, task 8
- Files: `crates/hotline-app/src/updater.rs:272`, `ui/src/components/{UpdatesSection,ConnectProvider,Skills,Secrets,ErrorCard}.tsx`, `ui/src/wire.ts:525`
- Collides with: 10 (`Skills.tsx`), 33
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
