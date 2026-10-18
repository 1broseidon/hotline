---
id: task-161
title: "P2-D-1 (E): wire.md matches the code"
column: todo
position: 50
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:07.403Z"
relatedFiles:
  - docs/wire.md
updatedAt: "2026-10-18T05:06:53.894Z"
contract:
  status: ready
  validation:
    commands:
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-138
  - task-139
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:23.254Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:06:53.894Z"
assignee: claude
---

## Description
Phase 2 group E (Docs), plan row D-1 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**wire.md matches the code.** The four seat rules and "every seat"; rows for the forty missing commands; the `call` target, `window`, the subscription cap, snapshot windows, capabilities, `computer_pull`; the short param lists

- Source: tests-and-docs D2–D4, tasks 10 and 11
- Files: `docs/wire.md`
- Collides with: 29 (cites `wire/mod.rs` lines): cite paths after 29 lands
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
