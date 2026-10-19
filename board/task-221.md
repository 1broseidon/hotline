---
id: task-221
title: "Subagents are clones of the home agent: same permission set, a focused start, reaching back with the same tools"
column: todo
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
updatedAt: "2026-10-19T23:20:46.787Z"
contract:
  status: ready
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
