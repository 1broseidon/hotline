---
id: task-166
title: "P2-D-6 (E): `docs/standards.md`"
column: in-progress
position: 55
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:10.035Z"
relatedFiles:
  - docs/standards.md
  - AGENTS.md
updatedAt: "2026-10-18T06:35:43.939Z"
contract:
  status: in_progress
  validation:
    commands:
      - test -s docs/standards.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T06:35:43.939Z"
    reworkCount: 0
dependsOn:
  - task-165
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:26.653Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T06:35:23.419Z"
assignee: claude
---

## Description
Phase 2 group E (Docs), plan row D-6 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`docs/standards.md`** from the [Standards](#standards) section below, once George approves it

- Source: this file
- Files: `docs/standards.md` (new)
- Collides with: D-5
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
