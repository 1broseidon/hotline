---
id: task-227
title: A job's row and thread can Cancel the job, and Stop a firing that is running
column: todo
position: 3
priority: medium
tags:
  - jobs
  - ui
assignee: claude
relatedFiles:
  - ui/src
  - crates/hotline-core/src/session
parentId: epic-20
createdAt: "2026-10-19T23:31:32.527Z"
updatedAt: "2026-10-19T23:58:29.745Z"
---

## Description
Gaps task-226 left (lead, 2026-10-19 18:30): cancelling a job is only in Schedules, and a firing in progress can't be stopped. The Jobs group's row and the job's thread header offer Cancel (ends the job with its final post, as ADR 0008 §7 says) and, while a firing runs, Stop (stops that firing only; the job keeps its schedule, and its thread says it was stopped). Under the same owner checks as schedule.cancel. UI tests for both; a harness test that Stop ends the firing's turn and the next firing still runs.

From task-221 (landed on release/0.41.0), two job-thread gaps that belong here, since this task already touches the row and the thread:
- The Jobs group does not show a job's `without` (ScheduledJob.without, contract Ability: computer, folders, mcp_servers). Show what a job runs without on its row or thread header, so the person can see a narrowed clone.
- A job's request_human card push names no thread (notify_in gets None), so tapping it opens the DM, not the job's thread. Pass the job's ThreadId so the push opens jobs/<id>.

How to work: run make check in the foreground with a generous timeout and wait for it to finish; never end the turn while it runs. Commit before ending the turn. Before saying done, merge release/0.41.0 into the branch and re-run make check, so the landing's check is incremental.
