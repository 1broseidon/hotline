---
id: task-47
title: "Core: make model and thread lists read each source once"
column: backlog
position: 30
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
updatedAt: "2026-10-14T20:05:27.659Z"
---

## Description
Split from task-19, preserving its BRO-239 scope. Compute connections once per models.list call; avoid repeated OS-secret and room reads. Scan threads once per list, and avoid loading unrelated complete side streams. If secret caching is added, invalidate on every vault mutation and revocation. Measure calls and returned results before and after; exercise the real list handlers with multiple teammates. Start with task-19's shared fold cache to avoid redundant caches.
