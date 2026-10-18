---
id: task-139
title: "P2-30 (C): `wire/commands.rs`:"
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:55.925Z"
relatedFiles:
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/wire/commands/persona.rs
  - crates/hotline-core/src/wire/commands/settings.rs
  - crates/hotline-core/src/wire/commands/models.rs
updatedAt: "2026-10-18T04:38:04.546Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/wire/commands.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:18:18.581Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:58.637Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:18:06.340Z"
    ended: "2026-10-18T04:38:04.546Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T04:38:03.987Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 30 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/commands.rs`:** `run` only dispatches, and the arms go to `commands/{persona,settings,models}.rs`

- Source: structure F7, task 16
- Files: `wire/commands.rs`
- Collides with: 41 (`mobile_persona_create`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:38:03.433Z: Landed on release/0.40.0
