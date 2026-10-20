---
id: task-229
title: A skipped slot doesn't count toward a loop's firing count
column: review
position: 0
priority: medium
tags:
  - jobs
  - qa
  - "0.41"
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session/schedule.rs
  - crates/hotline-core/src/session/job_thread.rs
  - crates/hotline-core/tests/schedule.rs
createdAt: "2026-10-20T02:04:25.300Z"
landsOn: release/0.41.0
updatedAt: "2026-10-20T02:15:15.007Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-20T02:04:26.987Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T02:04:27.936Z"
---

## Description
Found in Pork Chop's 0.41 re-test (2026-10-20 02:03, release/0.41.0 at f041c81c): "every 1 minute, sleep 70 then post the time, end after 3 firings" ran firing 1, skipped the next slot (overlap), then told firing 2 it was the job's last run and ended. A skipped slot counted toward the loop's count, so the person got 2 runs, not 3. A skip is not a firing: only firings that ran count toward a loop's count (the skip notice stays as it is). Harness test: a 3-firing loop whose first firing overruns one slot still runs three firings, and the third is told it is the last.

## Log
- 2026-10-20T02:04:26.243Z: [claude] For the agent: run make check in the foreground and wait for it, commit before ending your turn, and merge release/0.41.0 into the branch and re-run make check before saying done.
- 2026-10-20T02:15:14.691Z: [claude] Fixed on dispatch/task-229: fire_job now says whether the firing ran, and finish spends a loop's count (and ends it by count) only for a firing that ran; a skipped slot just moves to the next slot, and only the end date can end it. The skip notice is unchanged. Harness test a_skipped_slot_does_not_count_toward_a_loops_count (3-firing loop, every 2.5s, each firing stalls 4s: three firings, the third told it is the last; it failed with 2 before the fix), a unit test in schedule.rs, docs/sessions.md updated. make check passes; release/0.41.0 was already merged (f041c81c), nothing new to merge.
