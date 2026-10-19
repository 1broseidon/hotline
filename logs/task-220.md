---
id: task-220
title: "Jobs are subagents on a schedule: one schema, a thread per job, a running note"
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
updatedAt: "2026-10-19T23:20:11.483Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T22:24:09.237Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T23:18:24.410Z"
    duration: 3255
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T22:23:58.781Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T22:24:00.605Z"
    ended: "2026-10-19T23:18:30.071Z"
    outcome: review
completedAt: "2026-10-19T23:20:11.483Z"
---

## Description
ADR 0008 §7, the core of it: one schema for subagents and jobs (task, trigger now/at/every, report policy, optional end) written by the schedule/loop and subagent tools; each job an arm with its own thread under the home thread; each firing a turn on that thread, never the home thread; parked between firings; a running note rewritten each firing plus the last few firings; an overlapping firing skipped, said on the job's thread, with a suggested interval; ends on cancel or goal with a final post (through Room::post / post_to_chat, task-216/217); a plain reminder is one post with no arm; the person can talk to a job's thread. Today's quiet-run path in the home thread is removed (the quiet run on the DM's turns, Session::begin's quiet_run, and what is left of the quiet window that only it used). Existing schedules keep firing across the upgrade (migrated to job threads). Harness tests for each rule, in crates/hotline-core/tests/schedule.rs. Contract: carry the job listing on the wire (name, next run, last post, ended) so the desktop can draw it; the Jobs group itself is task-226, so leave the UI alone beyond what the contract change forces to compile. The clone model for the job's lease and start (§0) is task-221: run jobs on today's run lease here and say in a note what task-221 must change.

From task-217 and task-219 (landed on release/0.41.0): the posting gate is in session/post.rs. Room::open_posting opens it once per turn, from three places. In dm.rs it is the DM's turn, and it carries the job (held.quiet_run) when the turn is a quiet run. In turns.rs it is Threads::turn, for work threads and runs, and it passes None. In peers/delivery.rs it is a pair's exchange turn. When a job's firing moves off the DM onto its own thread, open its Posting in Threads::turn with the job, so that its post_to_chat, a file or an image still uses up the turn's one post and can wake the person once. Room::take_post (the single count shared by post_to_chat, send_file and generate_image) and Room::may_send must keep working for the job's thread; the harness tests tests/mcp/post_to_chat.rs *_quiet_runs_file_is_its_one_post and the ACP quiet-run post test must still pass, moved to the job thread. Subagent runs are refused post_to_chat, send_file and generate_image today (session/tests/posting.rs). A job is a subagent on a schedule, so make sure the job's thread is let through and a plain subagent run is still refused.

Run `make check` in the foreground with a generous timeout and wait for it to finish. Never end the turn while it is still running.

## Log
- 2026-10-19T23:18:15.371Z: [claude] Done on dispatch/task-220 (b5286499, e5c3297f, plus a tool-text fix). Jobs fire on their own thread, ThreadKind::Job in jobs/<id>.jsonl, through session/job_thread.rs (LiveJob on Room::run_queue). One schema is session::Brief (task, Trigger Now/At/Every, Report, Ends goal/date/count), written by the subagent, schedule and loop tools. The agent is parked after each firing and starts from a running note read off its thread. An overlapping firing is skipped with a notice on the thread, and the person is told once after two skips. Cancel posts where the job stood; a goal end is cancel_schedule with a report. A reminder is one post with no arm. The person can talk to a job via thread.prompt. Old schedules fire on job threads unchanged. Removed: the DM quiet run, session/quiet.rs, Session.quiet/pending_scheduled, prompt_scheduled, Wired.scheduled. Posting: Threads::turn opens Posting with the job; a job's file/image takes the turn's one post; a run is still refused. make check, make precheck (all 8 legs), the schedule harness (12 tests) and the quiet_run grep all pass.
- 2026-10-19T23:18:19.193Z: [claude] For task-221 (clone model): today a job runs on the run lease, lease_of(ThreadKind::Job, room.capability_lease(persona)), a scoped child made in Room::job_agent and revoked in LiveJob::let_go. Its tools are JOB_TOOLS in mcp/server.rs: the run's three plus post_to_chat, send_file, generate_image, list_teammates, list_schedules and cancel_schedule. Its Policy row (thread/policy.rs, ThreadKind::Job) is Computer::No and Tools::Job. task-221 must: give it the home thread's permission set and the computer (Computer::Lease); widen JOB_TOOLS, routing request_human cards and message_teammate replies to the job thread (both are keyed by side id today via TeammateTools.work); choose its model, since a job uses the persona's recorded model rather than the session's live one as runs do (on_the_sessions_model); and run subagents on ACP too, since Brief::subagent is written but runner.rs still refuses non-Hotline teammates and ignores trigger and report.
- 2026-10-19T23:18:22.488Z: [claude] For task-226 (Jobs group): the {"schedules": personaId} view sends ScheduleEntry {name, nextAt, lastPost{at,text}, endedAt, reminder, quiet}, with ended jobs included; schedule.cancel on an ended job removes it from the list. The thread id is {kind:"job", key: job id}. thread.list includes job threads (live, parked or closed). thread.prompt, cancel and answer work on a job; thread.close cancels it, and a phone may read a job but not prompt or close it. The only UI changes are dock.ts (job rank 5, word "Job"; powersOf(job) is still READ) and room.ts, whose fold now skips ended jobs. Left unwired: the search index rebuild (hotline-store search.rs threads_on) does not find job threads, since they have no tape link, so only live writes are indexed. The overrun count and its told-once flag are in memory. goal, until and times can be set by the loop tool only, not by schedule.create on the wire.
- 2026-10-19T23:20:11.105Z: Landed on release/0.41.0
