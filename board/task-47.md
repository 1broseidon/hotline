---
id: task-47
title: "Core: make model and thread lists read each source once"
column: in-progress
position: 1
description: Split from task-19, preserving its BRO-239 scope. Compute connections once per models.list call; avoid repeated OS-secret and room reads. Scan threads once per list, and avoid loading unrelated complete side streams. If secret caching is added, invalidate on every vault mutation and revocation. Measure calls and returned results before and after; exercise the real list handlers with multiple teammates. Start with task-19's shared fold cache to avoid redundant caches.
priority: medium
tags:
  - performance
  - core
relatedFiles:
  - crates/hotline-core/src/vault.rs
  - crates/hotline-core/src/thread/store.rs
  - crates/hotline-core/src/session/sides.rs
parentId: epic-5
createdAt: "2026-10-12T04:20:49.480Z"
updatedAt: "2026-10-21T10:50:22.283Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T10:49:56.887Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T10:49:58.658Z"
assignee: codex
---

## Description
Split from task-19, preserving its BRO-239 scope. Compute connections once per models.list call; avoid repeated OS-secret and room reads. Scan threads once per list, and avoid loading unrelated complete side streams. If secret caching is added, invalidate on every vault mutation and revocation. Measure calls and returned results before and after; exercise the real list handlers with multiple teammates. Start with task-19's shared fold cache to avoid redundant caches.

## Log
- 2026-10-21T10:49:55.251Z: [claude] 2026-10-21 Pork Chop: lands on dispatch/epic-5 (now dfb4048d: task-19, 46, 22, 20). Before done: merge dispatch/epic-5 into your branch, run make check in the foreground on the merged result, commit, and show a before/after measurement.
- 2026-10-21T10:57:57.113Z: [codex] Measured original dfb4048d handlers and updated handlers with the same real-wire fixture (3 teammates, 3 sides, 1 pair). Complete JSON results byte-identical: 17 models, 7 all threads, 3 Ada threads. models.list secret gets 3→1, room loads 4→1; thread.list whole-history loads 31→8 all / 13→4 Ada, unrelated side copies 2→0. Uses Log fold projection, request-local model sources only (no persistent secret cache). dispatch/epic-5 merge is up to date; foreground make check running.
