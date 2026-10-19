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
dependsOn:
  - task-217
  - task-218
  - task-219
updatedAt: "2026-10-19T20:03:14.442Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/tests/schedule.rs
      description: a harness test for each rule of ADR 0008 §7, and a schedule written before the upgrade firing on its job thread after it
    - type: docs
      path: docs/sessions.md
      description: jobs as subagents on a schedule, replacing the Quiet section
  validation:
    commands:
      - make check
      - cargo test -p hotline-core --test schedule
      - "! git grep -n quiet_run -- crates/hotline-core/src/session"
  metrics:
    readyAt: "2026-10-19T20:03:14.442Z"
---

## Description
ADR 0008 §7, the core of it: one schema for subagents and jobs (task, trigger now/at/every, report policy, optional end) written by the schedule/loop and subagent tools; each job an arm with its own thread under the home thread; each firing a turn on that thread, never the home thread; parked between firings; a running note rewritten each firing plus the last few firings; an overlapping firing skipped, said on the job's thread, with a suggested interval; ends on cancel or goal with a final post (through Room::post / post_to_chat, task-216/217); a plain reminder is one post with no arm; the person can talk to a job's thread. Today's quiet-run path in the home thread is removed (the quiet run on the DM's turns, Session::begin's quiet_run, and what is left of the quiet window that only it used). Existing schedules keep firing across the upgrade (migrated to job threads). Harness tests for each rule, in crates/hotline-core/tests/schedule.rs. Contract: carry the job listing on the wire (name, next run, last post, ended) so the desktop can draw it; the Jobs group itself is task-226, so leave the UI alone beyond what the contract change forces to compile. The clone model for the job's lease and start (§0) is task-221: run jobs on today's run lease here and say in a note what task-221 must change.
