---
id: task-220
title: "Jobs are subagents on a schedule: one schema, a thread per job, a running note"
column: todo
position: 16
priority: high
tags:
  - threads
  - jobs
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-contract
  - docs/sessions.md
parentId: epic-20
createdAt: "2026-10-19T19:59:32.588Z"
---

## Description
ADR 0008 §7, all of it: one schema for subagents and jobs (task, trigger now/at/every, report policy, optional end) written by the schedule/loop and subagent tools; each job an arm with its own thread under the home thread, listed in a Jobs group with its next run; each firing a turn on that thread, never the home thread; parked between firings; a running note rewritten each firing plus the last few firings; an overlapping firing skipped, said on the job's thread, with a suggested interval; ends on cancel or goal with a final post; a plain reminder is one post with no arm; the person can talk to a job's thread. Today's quiet-run path in the home thread is removed. Existing schedules keep firing across the upgrade (migrated to job threads). Harness tests for each rule. Contract and desktop: the Jobs group in the thread list (a minimal row: name, next run, last post).
