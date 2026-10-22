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
  - crates/hotline-core/src/wire/mod.rs
createdAt: "2026-10-22T01:04:57.301Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T01:07:47.477Z"
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
---

## Description
Desk side of George's phone scrollback report (2026-10-21; phone fix is on the hotline-mobile board, 'A thread keeps its scrollback').

- wire/mod.rs:1131, 1055-1067: the phone send queue holds 256 frames, durable frames go in with try_send, and a full queue cancels the whole connection. A burst of tool calls with their updates and roster rows on a slow relay or cellular link can fill it. The phone reconnects and every tape is re-snapshotted.
- wire/mod.rs:2159-2166: a lagging subscriber gets a second snapshot of its original window (40 lines for a background tape). The comment expects the client to fold by id, but the phone replaces.

Done when:
- A full phone queue drops or coalesces tool-call update frames before it cancels the connection: named test in wire (a burst of 1000 tool updates keeps a slow phone connected).
- A lagging phone is re-snapshotted with its full phone window: named test.
- make check passes.
