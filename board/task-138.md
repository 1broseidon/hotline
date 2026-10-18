---
id: task-138
title: "P2-29 (C): `wire/mod.rs`"
column: in-progress
position: 27
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:55.411Z"
relatedFiles:
  - crates/hotline-core/src/wire.rs
  - crates/hotline-core/src/wire/room_handle.rs
  - crates/hotline-core/src/wire/seat.rs
  - crates/hotline-core/src/wire/door.rs
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/subscribe.rs
  - crates/hotline-core/src/wire/phone.rs
updatedAt: "2026-10-18T04:17:24.473Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/wire.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:17:24.473Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:57.496Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:17:15.079Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 29 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/mod.rs`** → `room_handle.rs`, `seat.rs`, `door.rs`, `socket.rs`, `subscribe.rs`, `phone.rs`

- Source: structure F5, task 15
- Files: `wire.rs`, `wire/*`
- Collides with: 41 (`subscribe`, `stream_events`), 42 (`wire/mod.rs:1021`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
