---
id: task-150
title: "P2-41 (D): Argument groups"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:01.678Z"
relatedFiles:
  - crates/hotline-core/src/wire/subscribe.rs
  - crates/hotline-core/src/voice/call.rs
  - crates/hotline-core/src/voice/synthesis.rs
  - crates/hotline-core/src/wire/commands/persona.rs
  - crates/hotline-core/src/session/exchanges.rs
  - crates/hotline-core/src/remote.rs
  - crates/hotline-core/src/driver/rig/turn.rs
updatedAt: "2026-10-18T09:48:54.187Z"
contract:
  status: in_progress
  validation:
    commands:
      - "! git grep -n 'too_many_arguments' -- crates"
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T09:33:09.304Z"
    reworkCount: 0
dependsOn:
  - task-149
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:10.999Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T09:32:58.086Z"
    ended: "2026-10-18T09:48:54.186Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T09:48:53.799Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 41 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Argument groups.** Nine `too_many_arguments` refactors, one per function chain, with `make verify` for wire, voice and remote

- Source: lints, task 5
- Files: `wire/subscribe.rs` (was `wire/mod.rs:1903,2270`), `voice/{call,synthesis}.rs`, `wire/commands/persona.rs`, `session/exchanges.rs:176`, `remote.rs:737`, `driver/rig/turn.rs:689`
- Collides with: 28–31 (land first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T09:48:53.421Z: Landed on release/0.40.0
