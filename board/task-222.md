---
id: task-222
title: 0.41 lands together, and Pork Chop's QA on the Linux QA computer before George's
column: todo
position: 18
priority: high
tags:
  - qa
  - "0.41"
assignee: claude
relatedFiles:
  - docs
  - CHANGELOG.md
parentId: epic-20
createdAt: "2026-10-19T19:59:33.586Z"
dependsOn:
  - task-219
  - task-218
  - task-221
  - task-202
  - task-205
  - task-227
updatedAt: "2026-10-20T00:13:11.844Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: one harness proving epic-20's pieces together over the wire
    - type: docs
      path: CHANGELOG.md
      description: Unreleased says what 0.41 changed
  validation:
    commands:
      - make check
      - make precheck
      - make verify
  metrics:
    readyAt: "2026-10-19T20:03:15.032Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-20T00:13:09.901Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T00:13:11.844Z"
---

## Description
The epic's wiring and proof, last. After every 0.41 child (this epic's and epic-17's task-202 and task-205): first a headless harness in crates/hotline-core/tests/ (threads.rs or a new one) that drives the real core over the wire through the whole of epic-20 together, against its description of done: one teammate on each driver; post_to_chat from a quiet job, a work thread, a pair and an ACP teammate, each landing in the chat stamped with where it came from; a loud DM turn and a subagent refused; a file from a work thread landing in the chat; a recurring job on its own thread with its running note, a skipped overlap and a final post on cancel; a subagent reaching back with search_thread under the home thread's lease. Fix any seam between the slices it finds. Then make check, make precheck and CI (ci.yml and workspace-isolation.yml dispatched on release/0.41.0) green; docs (sessions.md, threads.md, security.md, design.md) and the CHANGELOG's Unreleased section say what changed. Then Pork Chop runs the desk from release/0.41.0 on the Linux QA computer (its prepared workspace, xAI sign-in) and exercises: post_to_chat from a quiet job, a work thread and an ACP teammate; a file from a work thread landing in the chat with its stamp; a recurring job (its thread, note, a skipped overlap and the suggested interval, a post on significance, cancel with a final post); a one-off job and a plain reminder; the Jobs group rows, with Cancel, and Stop on a running firing (the thread says Stopped, the next firing still runs) and a narrowed job's "Runs without …"; a subagent reaching back with search_thread; the team lead prompt, badge and single-teammate auto-lead. Screenshots of anything wrong; fix or file before handing George the QA checklist.

Seams task-221 left in the core (fix them here, with the harness proving each):
- A job delivery (a request_human answer or a message_teammate reply routed through Room::deliver_into_job) written just before a restart is not re-queued at startup, as a side's is. Re-queue it, or send it to the DM if the job no longer lives.
- The person's schedule.create on the wire takes no `without`, so only the home agent can narrow a job. Either accept it on the wire (with the same never-wider rule and a refused test) or state in design.md why the person narrows through the agent.
Out of scope, stated in the CHANGELOG as not yet wired: nothing starts a Hotline run for an ACP teammate yet (voice's work tool is the caller, epic-21), and an ACP harness's own subagents are still mirrored; and (from task-227) a job card's push now carries data.thread {kind,key}, but the phone app (hotline-mobile src/push.ts glanceOf) ignores it, so a tap still opens the teammate until the mobile repo reads it.

How to work: run make check in the foreground with a generous timeout and wait for it to finish; never end the turn while it runs. Commit before ending the turn. Before saying done, merge release/0.41.0 into the branch and re-run make check, so the landing's check is incremental.

## Log
- 2026-10-19T20:00:42.947Z: [claude] Split of work: the dispatched agent does the land-together part (check, precheck, CI, docs, CHANGELOG) and writes the QA checklist into epic-20's log; the hands-on QA on the Linux QA computer is Pork Chop's, from the watch loop, since only Pork Chop drives that computer. Hand-off to George waits for that QA.
- 2026-10-19T23:20:47.352Z: [claude] Lead, from task-220's handoff: a job's overrun count and its told-once flag live in memory (lost on restart), and goal/until/times can be set only by the loop tool, not schedule.create on the wire. Check both against the epic's done in the harness and QA; fix or file. The search rebuild missing job threads is task-221's.
- 2026-10-19T23:29:42.504Z: [claude] Lead, from task-226's handoff: a running job's row has no Cancel (cancel stays in Schedules), there is no Archive on a job thread, and there is no Stop on a job's turn. In QA, check that a person can cancel a running job and stop a runaway firing from the desk. If they can't, fix it or file it.
