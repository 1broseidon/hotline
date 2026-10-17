---
id: task-138
title: "P2-29 (C): `wire/mod.rs`"
column: todo
position: 27
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:55.411Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 29 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/mod.rs`** → `room_handle.rs`, `seat.rs`, `door.rs`, `socket.rs`, `subscribe.rs`, `phone.rs`

- Source: structure F5, task 15
- Files: `wire.rs`, `wire/*`
- Collides with: 41 (`subscribe`, `stream_events`), 42 (`wire/mod.rs:1021`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
