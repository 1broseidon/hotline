---
id: task-226
title: "The Jobs group in the thread list: a row per job with its name, next run and last post"
column: review
position: 12
priority: medium
tags:
  - threads
  - jobs
  - ui
relatedFiles:
  - ui/src
  - ui/src/generated/contract.ts
  - docs/adr/0008-one-agent-thread-layer.md
parentId: epic-20
dependsOn:
  - task-220
contract:
  status: delivered
  deliverables:
    - type: file
      path: ui/src
      description: the Jobs group and its row in the thread list
  validation:
    commands:
      - make check
  metrics:
    readyAt: "2026-10-19T20:02:36.231Z"
    pickedUpAt: "2026-10-19T23:21:04.426Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T23:28:00.278Z"
    duration: 416
createdAt: "2026-10-19T20:02:36.243Z"
updatedAt: "2026-10-19T23:28:00.278Z"
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-19T23:20:51.985Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T23:20:56.037Z"
assignee: claude
---

## Description
ADR 0008 §7, the desktop half, split from task-220. task-220 puts each job on its own thread under the home thread and carries the job listing on the wire; this draws it. In the window's thread list, a Jobs group beside the person's threads: one minimal row per job (name, next run, last post), opening the job's thread, where the person can talk to it. A job that ended stays listed as readable, marked ended. Posts from a job in the chat show "From a scheduled job" from the agent event's `from` (task-216). Read the contract task-220 left; add no core behaviour here, and if the wire lacks something the row needs, say so in a note rather than reshaping the core. Regenerate ui/src/generated/contract.ts only with an unfiltered `cargo test -p hotline-core` (AGENTS.md rule 3).

What task-220 left (landed on release/0.41.0): the {"schedules": personaId} view sends ScheduleEntry {name, nextAt, lastPost{at,text}, endedAt, reminder, quiet}, ended jobs included; schedule.cancel on an ended job removes it from the list (use it to dismiss an ended row). A job's thread id is {kind:"job", key: job id}. thread.list includes job threads (live, parked or closed). thread.prompt, cancel and answer work on a job; thread.close cancels it; a phone may read a job but not prompt or close it. task-220's only UI changes: dock.ts (job rank 5, word "Job"; powersOf(job) is still READ, so check whether the desktop needs prompt power on a job thread for the person to talk to it, and set it for the desktop only) and room.ts, whose fold now skips ended jobs (the Jobs group must still show them, marked ended).

How to work: run make check in the foreground with a generous timeout and wait for it to finish; never end the turn while it runs. Commit before ending the turn. Before saying done, merge release/0.41.0 into the branch and re-run make check, so the landing's check is incremental.

## Log
- 2026-10-19T23:27:59.973Z: [claude] Jobs group in the Dock thread list (ui only, no core change): useJobs subscribes {schedules: personaId}; jobRows orders running by nextAt then ended newest first, reminders left out (no thread). Row = name, next run/ended, last post; opens {kind:"job",key:id}; ended rows get a Remove (schedule.cancel). groupThreads now drops job threads so they appear once. powersOf(job): say for the desktop until closed; no Archive on a job (thread.close would cancel it; cancel stays in Schedules). Agent posts with from.kind "job" show "From a scheduled job". Left unwired: no Cancel button on a running job row, and no Stop on a job turn. make check passes after merging release/0.41.0; on branch dispatch/task-226.
