---
id: task-184
title: "Teammate card: the threads list reads thread.list, not peers.list"
column: review
position: 10
priority: high
tags:
  - threads
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:55.296Z"
relatedFiles:
  - ui/src/components/teammate/Threads.tsx
updatedAt: "2026-10-19T00:45:04.806Z"
contract:
  status: delivered
  validation:
    commands:
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:44:13.581Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:44:58.804Z"
    duration: 45
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
    ended: "2026-10-19T00:45:04.806Z"
    outcome: review
assignee: claude
---

## Description
Slice of task-35. ui/src/components/teammate/Threads.tsx:26 still asks peers.list. Move it to thread.list (the desk speaks threads2), with peers.list kept only as the fallback for an older desk. The markers and aliases stay in task-35.

Split from task-35, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:44:58.475Z: [claude] Threads.tsx now reads thread.list (pair rows only, name from withName or the roster, Threads takes a roster prop), falling back to peers.list if the command fails. Markers/aliases remain in task-35. make ui-check passes. On branch dispatch/task-184.
