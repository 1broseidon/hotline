---
id: task-229
title: A skipped slot doesn't count toward a loop's firing count
column: todo
position: 1
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
updatedAt: "2026-10-20T02:04:25.772Z"
---

## Description
Found in Pork Chop's 0.41 re-test (2026-10-20 02:03, release/0.41.0 at f041c81c): "every 1 minute, sleep 70 then post the time, end after 3 firings" ran firing 1, skipped the next slot (overlap), then told firing 2 it was the job's last run and ended. A skipped slot counted toward the loop's count, so the person got 2 runs, not 3. A skip is not a firing: only firings that ran count toward a loop's count (the skip notice stays as it is). Harness test: a 3-firing loop whose first firing overruns one slot still runs three firings, and the third is told it is the last.
