---
id: task-184
title: "Teammate card: the threads list reads thread.list, not peers.list"
column: in-progress
position: 10
priority: high
tags:
  - threads
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:55.296Z"
relatedFiles:
  - ui/src/components/teammate/Threads.tsx
updatedAt: "2026-10-19T00:44:13.581Z"
contract:
  status: in_progress
  validation:
    commands:
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:44:13.581Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-19T00:43:56.467Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:43:58.000Z"
assignee: claude
---

## Description
Slice of task-35. ui/src/components/teammate/Threads.tsx:26 still asks peers.list. Move it to thread.list (the desk speaks threads2), with peers.list kept only as the fallback for an older desk. The markers and aliases stay in task-35.

Split from task-35, which keeps the rest and stays deferred.
