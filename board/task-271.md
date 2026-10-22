---
id: task-271
title: A cancelled shell job can run its next command before it dies
column: todo
position: 2
priority: high
tags:
  - bug
  - jobs
  - 0.43.0
relatedFiles:
  - crates/hotline-core/tests/steering.rs
createdAt: "2026-10-22T07:37:09.827Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T07:37:21.018Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T07:37:18.021Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 93526
    at: "2026-10-22T07:37:21.018Z"
assignee: codex
---

## Description
Found overnight 2026-10-22 on the 0.43 draft PR #201 (macos job, run 39738536900): steering::an_operator_cancels_a_running_shell_before_its_ninety_second_wait_finishes failed at tests/steering.rs:319, assert!(!workspace.join("done").exists()).

The job is: echo started; (heartbeat loop) & sleep 90; echo done > done. The whole test binary finished in 60 s, so done can only exist if the cancel killed sleep and the parent sh survived long enough to run echo done > done. Cancellation must signal the tree in an order (or per pid rather than per process group) that lets the shell run its next statement. In a real job that could be 'sleep 5; deploy': a cancelled command runs its next step.

It passed on PR #200's macOS run, so it is a race, likelier on a loaded runner.

Done when:
- Cancelling a job signals the whole process group at once so no further statement of the job runs after cancel: a named test with 'sleep 30; touch after' that cancels during the sleep and asserts after never appears, looped 50 times without a failure on macOS and Linux.
- The steering test passes 20 consecutive runs on the Mac.
- make check passes.
