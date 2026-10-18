---
id: task-186
title: Start paired desks concurrently at launch
column: todo
position: 12
priority: high
tags:
  - startup
  - performance
parentId: epic-14
createdAt: "2026-10-18T23:40:56.313Z"
---

## Description
Slice of task-21. hotline-app desks.rs start_all starts paired desks one by one, so one slow desk delays the rest. Start them concurrently (join_all with a per-desk timeout), keeping per-desk errors separate. Test that a desk that hangs doesn't delay the others. The other startup work stays in task-21.

Split from task-21, which keeps the rest and stays deferred.
