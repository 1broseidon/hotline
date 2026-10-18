---
id: task-166
title: "P2-D-6 (E): `docs/standards.md`"
column: todo
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
updatedAt: "2026-10-18T00:15:26.787Z"
contract:
  status: ready
  validation:
    commands:
      - test -s docs/standards.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-165
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:26.653Z"
  until: done
  tries: 0
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
