---
id: task-55
title: Desk drops a phone's dead socket sooner
column: review
position: 3
priority: low
tags:
  - remote
createdAt: "2026-10-14T19:29:48.338Z"
updatedAt: "2026-10-19T00:31:29.292Z"
parentId: epic-14
relatedFiles:
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/phone.rs
  - crates/hotline-core/src/remote/v2.rs
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core
      description: "over the wire: a seat whose writes back up (a peer that stops reading) is closed by the desk within a bound"
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:20:30.457Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:31:24.572Z"
    duration: 654
dependsOn:
  - task-187
  - task-183
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:20:19.785Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:20:21.806Z"
    ended: "2026-10-19T00:31:29.292Z"
    outcome: review
assignee: claude
---

## Description
Seen 2026-10-14: after the phone left Wi-Fi for the Cloudflare tunnel, the desk kept the old LAN socket ESTABLISHED with ~20 KB queued until TCP gave up. The desk should notice on its own (ping/idle timeout on a seat whose writes back up) and close it.

0.40 triage (2026-10-18): Remaining: a 90 s silence timeout exists for phone links (wire/socket.rs:207), yet the Oct 14 socket stayed up. Find which seat it was, then close a seat whose writes back up (bounded send queue) or have the desk ping it. Over-the-wire test.

From task-187 (landed): wire/socket.rs now spawns 16 global reads (models.list, search.all, voice.status and others) in the background, as file reads already were, so their replies reach the outbound queue from spawned tasks as well as from the inline path. A bounded queue, or a close when writes back up, must cover those senders too: a spawned reply to a closed seat is dropped, never left blocking.

## Log
- 2026-10-19T00:31:24.040Z: [claude] Done on branch dispatch/task-55 (fb3d970c). Which seat: a direct (LAN) sealed phone seat. The phone app does name `lean` and pings every 25 s, and the 90 s timer was in that build, but the timer only runs while the socket loop waits for a frame. Before task-187, global reads like models.list ran inline and could hang, which kept the timer from running. And a writer stuck on a peer that stopped reading had no bound if the 256-frame queue never filled (quiet seat) or the seat wasn't lean. Fix: wire/socket.rs writer gives each frame WRITE_STALL (60 s) to reach the socket; past that it cancels the seat. That covers every seat and every sender, including the task-187 spawned replies, since they only enqueue (try_send on phones, unbounded on the desk), and a closed seat drops them. Test: remote/tests/phone.rs a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound (an owner that isn't lean, one 24 MB frame never read; the admission permit is released after the clock is advanced; fails without the change). docs/wire.md: outbox is 256 frames (it said 64) plus the 60 s bound. Left: an inline command that hangs still keeps the 90 s silence timer from running (the timer isn't selected during inline answers), and the desk only drops its fd. Unsent data goes to the kernel, with no SO_LINGER reset. fmt, clippy -D warnings, full cargo test -p hotline-core pass; contract.ts unchanged.
