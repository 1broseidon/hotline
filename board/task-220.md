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
updatedAt: "2026-10-19T22:23:57.824Z"
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

From task-217 and task-219 (landed on release/0.41.0): the posting gate is in session/post.rs. Room::open_posting opens it once per turn, from three places. In dm.rs it is the DM's turn, and it carries the job (held.quiet_run) when the turn is a quiet run. In turns.rs it is Threads::turn, for work threads and runs, and it passes None. In peers/delivery.rs it is a pair's exchange turn. When a job's firing moves off the DM onto its own thread, open its Posting in Threads::turn with the job, so that its post_to_chat, a file or an image still uses up the turn's one post and can wake the person once. Room::take_post (the single count shared by post_to_chat, send_file and generate_image) and Room::may_send must keep working for the job's thread; the harness tests tests/mcp/post_to_chat.rs *_quiet_runs_file_is_its_one_post and the ACP quiet-run post test must still pass, moved to the job thread. Subagent runs are refused post_to_chat, send_file and generate_image today (session/tests/posting.rs). A job is a subagent on a schedule, so make sure the job's thread is let through and a plain subagent run is still refused.

Run `make check` in the foreground with a generous timeout and wait for it to finish. Never end the turn while it is still running.
