---
id: task-50
title: "Window: cache room-wide picker and inspector lists per desk"
column: in-progress
position: 0
description: Split from task-22, preserving BRO-244. Room-wide model efforts, backends and capability reads are repeated on teammate switches. Share reads within a desk and invalidate on the relevant model/settings changes; never share credentials or data between desks. Keep per-persona computer status fresh. Verify picker stability and changed providers/settings, including remote desk switching; measure request counts on repeated teammate switches.
priority: medium
tags:
  - performance
  - ui
relatedFiles:
  - ui/src/components/Pickers.tsx
  - ui/src/components/Teammate.tsx
parentId: epic-5
createdAt: "2026-10-12T04:20:54.002Z"
updatedAt: "2026-10-21T10:50:19.116Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T10:49:57.147Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T10:50:01.259Z"
assignee: codex
---

## Description
Split from task-22, preserving BRO-244. Room-wide model efforts, backends and capability reads are repeated on teammate switches. Share reads within a desk and invalidate on the relevant model/settings changes; never share credentials or data between desks. Keep per-persona computer status fresh. Verify picker stability and changed providers/settings, including remote desk switching; measure request counts on repeated teammate switches.

## Log
- 2026-10-21T10:49:56.095Z: [claude] 2026-10-21 Pork Chop: lands on dispatch/epic-5 (now dfb4048d: task-19, 46, 22, 20). Before done: merge dispatch/epic-5 into your branch, run make check in the foreground on the merged result, commit, and show a before/after measurement.
