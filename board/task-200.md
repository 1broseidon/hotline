---
id: task-200
title: "Flaky: a process left running is sometimes never listed on the session (processes.rs, Linux isolation runner)"
column: backlog
position: 35
priority: medium
tags:
  - flaky
  - processes
  - tests
relatedFiles:
  - crates/hotline-core/tests/processes.rs
  - crates/hotline-core/tests/common/mod.rs
createdAt: "2026-10-19T10:27:43.228Z"
---

## Description
Seen once, 2026-10-19, workspace-isolation run 39232356092, check (ubuntu-24.04), on release/0.41.0 9e2c01bc: the teammate replied "Left it running." after `echo $$ > sleeper.pid; exec sleep 300`, but no session row with `processes` arrived within the 60 s next_where patience (crates/hotline-core/tests/processes.rs:221, common/mod.rs:198). The same commit passed it in ci's Linux job, three times locally, and on the re-run of the failed job. Likely a race between the shell job being handed to the session at park and the roster's session update, under the isolation runner's load. Find whether a session update can be skipped when the handover lands after the turn's last snapshot; fix the race, not the patience.

## Log
- 2026-10-20T02:16:28.728Z: [claude] Second instance, a different test, same shape (2026-10-20 02:05, workspace-isolation check (ubuntu-24.04) on release/0.41.0 f041c81c): threads.rs a_work_thread_is_one_conversation_to_old_and_threads2_clients waited 20 s in next_where for a frame that never came after the side thread's turn ended; passes 3/3 locally and in ci's Linux job. Likely the same root: an update dropped or reordered under the isolation runner's load. Look at both together.
- 2026-10-21T05:03:46.506Z: [claude] 2026-10-21: fixed threads::a_work_thread_is_one_conversation_to_old_and_threads2_clients (a subscription answers before it listens, so the tape's link can arrive in its snapshot; the test now reads it either way). One more unnamed failure seen once in 7 runs of the hotline-core integration binary on Linux; catch its name next time.
- 2026-10-21T06:58:50.609Z: [claude] 2026-10-21 Pork Chop: process_chip_output_is_the_latest_of_what_it_wrote (tests/processes.rs:329) timed out once in make check on the Mac under load (two Codex builds running): 'the output arrived while the command runs: Elapsed', with a wire 'Connection reset without closing handshake'. 5/5 alone in 1.5 s.
