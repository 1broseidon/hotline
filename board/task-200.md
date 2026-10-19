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
