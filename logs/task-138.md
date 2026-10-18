---
id: task-138
title: "P2-29 (C): `wire/mod.rs`"
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
updatedAt: "2026-10-18T04:49:37.951Z"
contract:
  status: delivered
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
    deliveredAt: "2026-10-18T04:38:44.574Z"
    duration: 1280
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
    ended: "2026-10-18T04:49:37.951Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T04:49:37.406Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 29 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/mod.rs`** → `room_handle.rs`, `seat.rs`, `door.rs`, `socket.rs`, `subscribe.rs`, `phone.rs`

- Source: structure F5, task 15
- Files: `wire.rs`, `wire/*`
- Collides with: 41 (`subscribe`, `stream_events`), 42 (`wire/mod.rs:1021`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:38:48.946Z: [claude] Split wire.rs (2,518 → 69 lines) per structure F5 into wire/{room_handle,seat,door,socket,subscribe,phone}.rs, with roster_entry/activity_on/waiting_on appended to wire/roster.rs; commit ed08b065 on dispatch/task-138. Move-only: the bodies match, and the only changes are pub(super) where a sibling or a test reaches an item, the sibling imports, roster's mod tests moved back to the file's end, and the federation doc link. Deviations: TAPE_PAGE/TAPE_PAGE_MAX/THROUGH_CONTEXT moved to subscribe.rs beside thread_page (their only user); Outbox/Outgoing/Refused/the code constants landed in socket.rs; the test-only snapshot_for_seat moved into wire/tests.rs fixtures; wire/tests.rs now imports the moved items explicitly (tests/*.rs unchanged). docs/security.md and docs/wire.md point Seat::permits at wire/seat.rs. For row 41: subscribe and stream_events are now in wire/subscribe.rs. For row 42: the result_large_err allow is on door.rs `serve` (was wire/mod.rs:1021). Nothing left unwired. make check and make precheck pass (all 7 legs); contract.ts unchanged.
- 2026-10-18T04:49:36.894Z: Landed on release/0.40.0
