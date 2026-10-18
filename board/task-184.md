---
id: task-184
title: "Teammate card: the threads list reads thread.list, not peers.list"
column: todo
position: 10
priority: high
tags:
  - threads
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:55.296Z"
---

## Description
Slice of task-35. ui/src/components/teammate/Threads.tsx:26 still asks peers.list. Move it to thread.list (the desk speaks threads2), with peers.list kept only as the fallback for an older desk. The markers and aliases stay in task-35.

Split from task-35, which keeps the rest and stays deferred.
