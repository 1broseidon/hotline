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
---

## Description
Gaps task-226 left (lead, 2026-10-19 18:30): cancelling a job is only in Schedules, and a firing in progress can't be stopped. The Jobs group's row and the job's thread header offer Cancel (ends the job with its final post, as ADR 0008 §7 says) and, while a firing runs, Stop (stops that firing only; the job keeps its schedule, and its thread says it was stopped). Under the same owner checks as schedule.cancel. UI tests for both; a harness test that Stop ends the firing's turn and the next firing still runs.
