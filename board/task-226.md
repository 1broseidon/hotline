---
id: task-226
title: "The Jobs group in the thread list: a row per job with its name, next run and last post"
column: todo
position: 12
description: "ADR 0008 §7, the desktop half, split from task-220. task-220 puts each job on its own thread under the home thread and carries the job listing (name, next run, last post) on the wire; this draws it. In the window's thread list, a Jobs group beside the person's threads: one minimal row per job (name, next run, last post), opening the job's thread, where the person can talk to it. A job that ended stays listed as readable, marked ended. Posts from a job in the chat show \"From a scheduled job\" from the agent event's `from` (task-216). Read the contract task-220 left; add no core behaviour here, and if the wire lacks something the row needs, say so in a note rather than reshaping the core. Regenerate ui/src/generated/contract.ts only with an unfiltered `cargo test -p hotline-core` (AGENTS.md rule 3)."
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
  status: ready
  deliverables:
    - type: file
      path: ui/src
      description: the Jobs group and its row in the thread list
  validation:
    commands:
      - make check
  metrics:
    readyAt: "2026-10-19T20:02:36.231Z"
createdAt: "2026-10-19T20:02:36.243Z"
---

## Description
ADR 0008 §7, the desktop half, split from task-220. task-220 puts each job on its own thread under the home thread and carries the job listing (name, next run, last post) on the wire; this draws it. In the window's thread list, a Jobs group beside the person's threads: one minimal row per job (name, next run, last post), opening the job's thread, where the person can talk to it. A job that ended stays listed as readable, marked ended. Posts from a job in the chat show "From a scheduled job" from the agent event's `from` (task-216). Read the contract task-220 left; add no core behaviour here, and if the wire lacks something the row needs, say so in a note rather than reshaping the core. Regenerate ui/src/generated/contract.ts only with an unfiltered `cargo test -p hotline-core` (AGENTS.md rule 3).
