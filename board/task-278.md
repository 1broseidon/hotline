---
id: task-278
title: A stalled phone seat is sometimes counted closed while its TCP connection is never reset (macOS)
column: todo
position: 2
priority: medium
tags:
  - bug
  - remote
  - flaky
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/remote/tests/phone.rs
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/transport.rs
createdAt: "2026-10-22T22:44:26.174Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T22:44:27.607Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T22:44:27.482Z"
  until: review
  tries: 0
assignee: codex
---

## Description
Follow-up to task-272 (2026-10-22). After task-272 bounded each read by 5 s instead of the whole drain, remote::tests::phone::a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound still failed on the GitHub macos job (PR #201 head a00b319d, run 39864325043, job 120122574283): phone.rs:861 'the stalled transport makes progress until it resets: Elapsed(())'. So after the desk's admission counts reached (0, 0), a read waited 5 s with neither data nor a reset: the seat was counted gone while its TCP stream was not yet (or never) closed abortively. Locally it passes 20/20 and earlier 15/15, so it needs load to show.

Likely: the seat's admission slot is released before the writer task that owns the stalled socket is dropped (it is parked in a write under the paused-clock trick or the AbortiveClose path in wire/transport.rs runs only when the write future ends), so the RST is sent late or not at all.

Done when:
- The seat's TCP stream is reset (SO_LINGER 0 close) before or atomically with the admission slot's release, so the count can never say closed while the socket is open: a named test that asserts the reset arrives within 1 s of the count reaching zero, run 200 times under parallel load (e.g. cargo test with --test-threads=32 and a CPU hog) on the Mac without a failure.
- The original test passes 50 consecutive runs under the same load.
- make check passes.
