---
id: task-181
title: "Agent registry: keep the last working list, refresh it in the background at session start, and say plainly when a model isn't supported"
column: todo
position: 7
priority: high
tags:
  - acp
  - registry
  - bugfix
parentId: epic-14
createdAt: "2026-10-18T23:40:53.687Z"
---

## Description
Slice of task-14 (items 2-4). driver/acp/registry.rs: 24 h TTL (:36), only the picker refreshes (backends(), desk.rs), session start uses cached_backends, and the hand-pinned codex-acp fallback sits at :198. (a) A failed registry fetch keeps the last known good list. (b) Session start refreshes in the background when the cache is stale, without blocking. (c) A 'model not supported by this harness version' failure becomes a card saying which harness is too old and what to do, not a raw 400. Leave auto-refresh-and-retry for later. Tests for each.

Split from task-14, which keeps the rest and stays deferred.
