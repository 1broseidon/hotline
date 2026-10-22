---
id: task-264
title: A burst of tool events doesn't drop the phone's connection
column: in-progress
position: 0
priority: medium
tags:
  - bug
  - remote
  - mobile
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/wire/queue.rs
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/subscribe.rs
  - crates/hotline-core/src/wire/roster.rs
  - crates/hotline-core/src/wire/phone.rs
  - crates/hotline-core/src/wire/tests/phone_pressure.rs
  - crates/hotline-core/tests/desk/connections.rs
  - docs/design.md
  - docs/wire.md
createdAt: "2026-10-22T01:04:57.301Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T01:17:28.848Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T01:07:15.418Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-22T01:07:17.375Z"
assignee: codex
decisions:
  - decision-32
---

## Description
Desk side of George's phone scrollback report (2026-10-21; phone fix is on the hotline-mobile board, 'A thread keeps its scrollback').

- wire/mod.rs:1131, 1055-1067: the phone send queue holds 256 frames, durable frames go in with try_send, and a full queue cancels the whole connection. A burst of tool calls with their updates and roster rows on a slow relay or cellular link can fill it. The phone reconnects and every tape is re-snapshotted.
- wire/mod.rs:2159-2166: a lagging subscriber gets a second snapshot of its original window (40 lines for a background tape). The comment expects the client to fold by id, but the phone replaces.

Done when:
- A full phone queue drops or coalesces tool-call update frames before it cancels the connection: named test in wire (a burst of 1000 tool updates keeps a slow phone connected).
- A lagging phone is re-snapshotted with its full phone window: named test.
- make check passes.

## Log
- 2026-10-22T01:10:14.285Z: [codex] Confirmed current paths are wire/socket.rs and wire/subscribe.rs. Keeping the bounded queue and write deadline; adding tool/delta eviction and roster-row coalescing (retaining negotiated catalogs), plus full PHONE_WINDOW recovery after log lag. Existing wire decisions remain in force.
- 2026-10-22T01:15:24.813Z: [codex] Four wire regression tests pass, including 1,000 tool updates with a stalled writer for companion and lean owner seats, full-window lag recovery, queue bounds and roster catalog coalescing. Review tightened recovery to Log.snapshot's atomic fold/receiver boundary so stale queued revisions cannot replay. make check is running; real-core lean recovery harness added.
- 2026-10-22T01:21:58.421Z: [codex] Committed f3be0e34 on dispatch/task-264. make check passes, including the 1,000-tool stalled-phone tests for companion/lean-owner seats, full-window phone recovery, bounded queue/catalog regressions, and real-core lag recovery over the wire. make precheck is running for the handoff.
- 2026-10-22T01:24:17.557Z: [codex] Ready for review on dispatch/task-264 at f3be0e34: the bounded remote queue sheds tool/delta frames and coalesces roster updates; lag recovery restores the full phone window with an atomic snapshot/receiver. Named wire regressions and the real-core harness pass. make check and all 8 make precheck legs pass; worktree clean. Next: review and merge the branch.
