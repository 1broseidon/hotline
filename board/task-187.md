---
id: task-187
title: Run read-only global commands without blocking the socket
column: todo
position: 13
priority: high
tags:
  - wire
  - performance
parentId: epic-14
createdAt: "2026-10-18T23:40:56.843Z"
relatedFiles:
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/tests.rs
updatedAt: "2026-10-18T23:53:19.124Z"
---

## Description
Slice of task-20. wire/socket.rs: per-teammate lanes exist (:231), but other commands are awaited inline (:244-269). Spawn the non-mutating global reads (models.list, search, voice.status and similar), as file reads already are (:445-464). Keep every mutating command, settings.update included, in order. Test that a slow read doesn't hold up the next command.

Split from task-20, which keeps the rest and stays deferred.
