---
id: task-278
title: A stalled phone seat is sometimes counted closed while its TCP connection is never reset (macOS)
column: review
position: 3
priority: medium
tags:
  - bug
  - remote
  - flaky
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/remote/channel.rs
  - crates/hotline-core/src/remote/tests/phone.rs
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/tests/phone_pressure.rs
  - crates/hotline-core/src/wire/transport.rs
  - docs/design.md
  - docs/security.md
createdAt: "2026-10-22T22:44:26.174Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T23:01:10.412Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T22:44:27.482Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 47499
    at: "2026-10-22T22:44:29.114Z"
assignee: codex
decisions:
  - decision-40
---

## Description
Follow-up to task-272 (2026-10-22). After task-272 bounded each read by 5 s instead of the whole drain, remote::tests::phone::a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound still failed on the GitHub macos job (PR #201 head a00b319d, run 39864325043, job 120122574283): phone.rs:861 'the stalled transport makes progress until it resets: Elapsed(())'. So after the desk's admission counts reached (0, 0), a read waited 5 s with neither data nor a reset: the seat was counted gone while its TCP stream was not yet (or never) closed abortively. Locally it passes 20/20 and earlier 15/15, so it needs load to show.

Likely: the seat's admission slot is released before the writer task that owns the stalled socket is dropped (it is parked in a write under the paused-clock trick or the AbortiveClose path in wire/transport.rs runs only when the write future ends), so the RST is sent late or not at all.

Done when:
- The seat's TCP stream is reset (SO_LINGER 0 close) before or atomically with the admission slot's release, so the count can never say closed while the socket is open: a named test that asserts the reset arrives within 1 s of the count reaching zero, run 200 times under parallel load (e.g. cargo test with --test-threads=32 and a CPU hog) on the Mac without a failure.
- The original test passes 50 consecutive runs under the same load.
- make check passes.

## Log
- 2026-10-22T22:47:19.469Z: [codex] Traced teardown: seated_inner currently joins the writer before setting zero linger, and sealed_session retains admission until it returns. Adding a named 1 s reset regression and stressing the current path before choosing the smallest fix; preserving decision-24's timeout-only reset.
- 2026-10-22T22:52:43.963Z: [codex] Baseline 1 s regression passed 200/200 with four parallel processes, --test-threads=32, and 12 CPU hogs (162.6 s), so the exact CI flake remains unreproduced. A deterministic cancellation regression fails before the fix: the detached writer sends 65,605 bytes after its seat task is cancelled and joined. Replacing the spawned writer with a locally polled future and consuming abortive close removes that lifetime gap.
- 2026-10-22T22:55:35.653Z: [codex] Committed 98dd1859 on dispatch/task-278: the seat polls its writer locally, cancellation drops both halves, and consuming zero-linger close happens without suspension before admission release. The deterministic writer-lifetime regression and all 14 phone tests pass. Decision-40 refines wire.abortive-close without expanding which closes reset; final loaded repetitions and make check are running.
- 2026-10-22T22:58:39.530Z: [codex] Loaded Mac acceptance passed on 98dd1859: a_stalled_seat_resets_tcp_within_one_second_of_releasing_admission 200/200 (191.8 s), original a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound 50/50 (47.6 s). Each used four concurrent test processes with --test-threads=32 and 12 captured-PID yes CPU hogs, also overlapping make check compilation/tests, on this 18-core arm64 Darwin 27 Mac. No failures; all hogs stopped by their captured PIDs. Logs: target/task-278/{reset-after,original-after}/.
- 2026-10-22T23:01:10.134Z: [codex] Ready for review on dispatch/task-278, commit 98dd1859; clean worktree. Writer lifetime and consuming abortive close now enforce transport teardown before admission release, with cancellation and 1 s reset regressions. Loaded Mac runs: new 200/200, original 50/50; make check passed and all 8 make precheck legs passed (including Windows). Exact prior CI flake remains unreproduced; the detached-writer cancellation failure was reproduced and fixed. Reviewer next: inspect ownership/close ordering and confirm macOS CI when merging.
