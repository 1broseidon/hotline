---
id: task-272
title: "Flaky on CI macOS: a stalled phone seat's TCP reset must arrive within 5 s while draining 24 MB"
column: todo
position: 2
priority: low
tags:
  - flaky
  - tests
  - remote
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/remote/tests/phone.rs
createdAt: "2026-10-22T08:26:53.752Z"
updatedAt: "2026-10-22T22:04:50.537Z"
landsOn: release/0.43.0
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T22:04:48.259Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 47499
    at: "2026-10-22T22:04:50.537Z"
assignee: codex
---

## Description
remote::tests::phone::a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound failed once on the GitHub macos job for PR #201 (2026-10-22, head 80e73e9b): 'the stalled transport resets: Elapsed(())' at phone.rs:867. It passed on the previous macOS run of the same code and 15/15 times locally. The test drains a 24 MB frame from the socket with a 5 s real-time timeout before the reset; a loaded runner can be slower.

Done when:
- The drain is bounded by bytes or progress (keep waiting while reads make progress), not a fixed 5 s, and the test still fails if the desk closes with a FIN instead of a reset: named test unchanged in intent.
- 20 consecutive runs pass on the Mac.

## Log
- 2026-10-22T22:04:47.473Z: [claude] Failed again on PR #201 head 6036a4d8 (macos job, run 39857519772, 2026-10-22 21:37 UTC): two of the last four macOS runs. Pulled into 0.43.
