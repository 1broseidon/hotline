---
id: task-221
title: "Subagents are clones of the home agent: same permission set, a focused start, reaching back with the same tools"
column: in-progress
position: 17
priority: high
tags:
  - threads
  - security
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-core/src/driver
  - docs/security.md
parentId: epic-20
createdAt: "2026-10-19T19:59:33.081Z"
dependsOn:
  - task-220
updatedAt: "2026-10-19T23:21:04.880Z"
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: a subagent and a job run under the home thread's lease (and a narrowed one when the home agent narrows it), allowed and refused, and reach back with search_thread and list_chapters
    - type: docs
      path: docs/security.md
      description: the clone model's default, grant source, enforcement points, tests and residual risk
  validation:
    commands:
      - make check
  constraints:
    - "A capability change: follow docs/security.md"
    - A clone's lease is never wider than the home thread's
  metrics:
    readyAt: "2026-10-19T20:03:14.742Z"
    pickedUpAt: "2026-10-19T23:21:04.880Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T23:20:50.726Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T23:20:52.686Z"
---

## Description
ADR 0008 §0 and §7: inventory what a subagent run gets today (its lease is scoped narrower than the home thread's) and bring runs and jobs to the clone model: the home thread's lease unless the home agent narrows it for the task, the home agent's model and workspace, a start from the brief (not the home thread), and search_thread, list_chapters and the thread readers to reach back. A capability change: security.md's five items, tests allowed and refused, residual risk stated.

From task-220 (landed on release/0.41.0). Jobs fire on their own thread (ThreadKind::Job, jobs/<id>.jsonl) through session/job_thread.rs (LiveJob on Room::run_queue); the one schema is session::Brief (task, Trigger Now/At/Every, Report, Ends), written by the subagent, schedule and loop tools. Today a job runs on the run lease: lease_of(ThreadKind::Job, room.capability_lease(persona)), a scoped child made in Room::job_agent and revoked in LiveJob::let_go. Its tools are JOB_TOOLS in mcp/server.rs: the run's three plus post_to_chat, send_file, generate_image, list_teammates, list_schedules and cancel_schedule. Its Policy row (thread/policy.rs, ThreadKind::Job) is Computer::No and Tools::Job. This task must:
- give runs and jobs the home thread's permission set and the computer (Computer::Lease), never wider than the home thread's lease, narrowed when the home agent narrows it;
- widen JOB_TOOLS to the clone's set, routing request_human cards and message_teammate replies to the job thread (both are keyed by side id today via TeammateTools.work);
- choose the model: a job uses the persona's recorded model, while runs use the session's live one (on_the_sessions_model); make both the home agent's model;
- run subagents on ACP too: Brief::subagent is written but runner.rs still refuses non-Hotline teammates and ignores trigger and report;
- reach back: search_thread must find job threads after a restart. The search index rebuild (hotline-store search.rs threads_on) misses them today, since job threads have no tape link, so only live writes are indexed. Fix the rebuild so a clone can search its own and the home thread's history.
Keep the posting rule: a job's thread may post (one per turn, Threads::turn's Posting carries the job); a plain subagent run is still refused post_to_chat, send_file and generate_image (session/tests/posting.rs).

How to work: run make check in the foreground with a generous timeout and wait for it to finish; never end the turn while it runs. Commit before ending the turn. Before saying done, merge release/0.41.0 into the branch and re-run make check, so the landing's check is incremental.

## Log
- 2026-10-19T23:56:14.043Z: [claude] Done on dispatch/task-221 (merged release/0.41.0, make check green). Runs and jobs are clones (ADR 0008 §0): clone_of (session/agent.rs) gives the home session's live model and effort and the record's permission set, narrowed by `without` (contract Ability: computer, folders, mcp_servers) on the subagent tool and the schedule/loop tools, kept on ScheduledJob.without (an unreadable list leaves everything out). Policy Run and Job rows are Computer::Lease. Leases unchanged and never wider: a run is scoped from the asking session, a job from the teammate's authority. RUN_TOOLS is now the reading set (search_thread, list_chapters, list_teammates, list_schedules, computer_status, web_search). JOB_TOOLS is gone: a job has the home thread's tools minus JOB_WITHOUT (chapter moves, react, open_link, schedule, loop). Cards and replies route by the tools' ThreadId: a job's request_human card is written on jobs/<id>.jsonl, human_card returns (card, thread) found by location, and answers and message_teammate replies (Request.reply_job) come back through Room::deliver_into_job while the job lives, else to the DM. runner.rs takes a Brief, runs on ACP too, refuses a non-Now/Asker brief, and uses the teammate's preamble plus a run brief. The search rebuild finds job threads from the room stream's jobs (jobs_of in hotline-store search.rs). Tests: tests/schedule/clones.rs (over the wire: subagent and job reach back, folders narrowed, job card round trip, search after an index rebuild), plus unit tests in runs.rs, scheduled.rs, agent.rs and search.rs. security.md has a new "Clones of the home agent" section and matrix row. Left unwired: nothing starts a Hotline run for an ACP teammate yet (voice slice 6's work tool is the caller; an ACP harness's own subagents are still mirrored); the person's schedule.create on the wire takes no `without`; the Jobs group does not show `without`; a job's card push names no thread (notify_in gets None, so it opens the DM); a job delivery written just before a restart is not re-queued at startup as a side's is (exchange replies are re-sent by exchange recovery).
