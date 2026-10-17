---
id: task-163
title: "P2-D-3 (E): threads.md and sessions.md name live code"
column: todo
position: 52
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:08.469Z"
relatedFiles:
  - docs/threads.md
  - docs/sessions.md
  - docs/adr/0005-front-is-a-full-agent.md
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - grep -q 'Accepted (0.39)' docs/adr/0005-front-is-a-full-agent.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
---

## Description
Phase 2 group E (Docs), plan row D-3 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**threads.md and sessions.md name live code** (`run_queue`, `Exchange::from_thread`, `threads2`); ADR 0005 becomes "Accepted (0.39)" with the 0.38.1 credit explained

- Source: tests-and-docs D8, D9, tasks 15 (threads part), 16
- Files: `docs/threads.md`, `docs/sessions.md`, `docs/adr/0005-front-is-a-full-agent.md`
- Collides with: none
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
