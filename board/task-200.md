---
id: task-200
title: "Flaky: a process left running is sometimes never listed on the session (processes.rs, Linux isolation runner)"
column: review
position: 1
priority: medium
tags:
  - flaky
  - processes
  - tests
relatedFiles:
  - crates/hotline-core/tests/processes.rs
  - crates/hotline-core/tests/common/mod.rs
createdAt: "2026-10-19T10:27:43.228Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T09:10:49.489Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T09:10:55.827Z"
assignee: codex
updatedAt: "2026-10-21T09:27:51.355Z"
landsOn: release/0.42.0
---

## Description
Seen once, 2026-10-19, workspace-isolation run 39232356092, check (ubuntu-24.04), on release/0.41.0 9e2c01bc: the teammate replied "Left it running." after `echo $$ > sleeper.pid; exec sleep 300`, but no session row with `processes` arrived within the 60 s next_where patience (crates/hotline-core/tests/processes.rs:221, common/mod.rs:198). The same commit passed it in ci's Linux job, three times locally, and on the re-run of the failed job. Likely a race between the shell job being handed to the session at park and the roster's session update, under the isolation runner's load. Find whether a session update can be skipped when the handover lands after the turn's last snapshot; fix the race, not the patience.

## Log
- 2026-10-20T02:16:28.728Z: [claude] Second instance, a different test, same shape (2026-10-20 02:05, workspace-isolation check (ubuntu-24.04) on release/0.41.0 f041c81c): threads.rs a_work_thread_is_one_conversation_to_old_and_threads2_clients waited 20 s in next_where for a frame that never came after the side thread's turn ended; passes 3/3 locally and in ci's Linux job. Likely the same root: an update dropped or reordered under the isolation runner's load. Look at both together.
- 2026-10-21T05:03:46.506Z: [claude] 2026-10-21: fixed threads::a_work_thread_is_one_conversation_to_old_and_threads2_clients (a subscription answers before it listens, so the tape's link can arrive in its snapshot; the test now reads it either way). One more unnamed failure seen once in 7 runs of the hotline-core integration binary on Linux; catch its name next time.
- 2026-10-21T06:58:50.609Z: [claude] 2026-10-21 Pork Chop: process_chip_output_is_the_latest_of_what_it_wrote (tests/processes.rs:329) timed out once in make check on the Mac under load (two Codex builds running): 'the output arrived while the command runs: Elapsed', with a wire 'Connection reset without closing handshake'. 5/5 alone in 1.5 s.
- 2026-10-21T09:10:48.163Z: [claude] 2026-10-21 Pork Chop, for the Codex run: two seen tonight. (1) process_chip_output_is_the_latest_of_what_it_wrote (tests/processes.rs:329) timed out once in make check on the Mac under heavy load, with a wire 'Connection reset without closing handshake'; 5/5 alone in 1.5 s. (2) an unnamed failure once in 7 runs of the Linux integration binary. Look for waits on snapshot-delivered events (the fix pattern in a663d732 / 2735d01a: a new subscription's snapshot can already hold the awaited event or an older one) and for fixed timeouts under load. Fix in tests or harness only unless a real product race is found; prove with repeated runs.
- 2026-10-21T09:11:39.427Z: [codex] Found two test races: the process-row test consumes a ready row with processes then waits for a second ready row, which need not exist; output test assumes stderr marker is drained after stdout, but independent pipes can place it before >16 KiB stdout and evict it. Tracing watcher confirms hold notifies independently of turn completion; preparing test-only fixes.
- 2026-10-21T09:14:50.792Z: [codex] Test-only fix now accepts roster snapshots and requires ready+process in a single observation; added a real-core snapshot regression. Output fixture gates stderr on observing the final stdout line, preserving both-pipe coverage without assuming drain order. Initial process suite passes. make check is blocked by existing Cargo.lock advisories RUSTSEC-2026-0381/0382/0383; running remaining checks separately.
- 2026-10-21T09:18:51.869Z: [codex] Committed e1e1ff8f on dispatch/task-200 (clean worktree). Test-only fixes: consume ready+process once, accept roster snapshots, and gate stderr until stdout is observed; added real-core snapshot regression. No product handover race found: hold notifies its own watcher, roster reads current state. Final process suite passed 31 consecutive runs (9 passed/1 ignored each), plus 30 earlier stress runs; cargo test --workspace and all 8 make precheck legs passed, as did UI/Python checks. make check stopped on existing Cargo.lock advisories RUSTSEC-2026-0381/0382/0383; remaining checks run separately. Linux runtime not rerun; unnamed Linux failure remains unidentified. Thread fix a663d732 is already recorded on the task and was not duplicated from its other branch. Handoff: review and merge this branch; address dependency advisories separately.
