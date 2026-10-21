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
updatedAt: "2026-10-21T10:55:44.352Z"
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
decisions:
  - decision-23
---

## Description
Split from task-22, preserving BRO-244. Room-wide model efforts, backends and capability reads are repeated on teammate switches. Share reads within a desk and invalidate on the relevant model/settings changes; never share credentials or data between desks. Keep per-persona computer status fresh. Verify picker stability and changed providers/settings, including remote desk switching; measure request counts on repeated teammate switches.

## Log
- 2026-10-21T10:49:56.095Z: [claude] 2026-10-21 Pork Chop: lands on dispatch/epic-5 (now dfb4048d: task-19, 46, 22, 20). Before done: merge dispatch/epic-5 into your branch, run make check in the foreground on the merged result, commit, and show a before/after measurement.
- 2026-10-21T10:52:59.952Z: [codex] Tracing confirms repeated models.efforts, backends.list, voice.status and capabilities.options on picker/inspector remounts. Implementing explicit metadata-only reads keyed by Wire identity and room change epoch, with synchronous reuse and coalesced promises; computer.status remains uncached.
- 2026-10-21T10:57:02.628Z: [codex] Before/after measured with the same real picker + inspector DOM fixture and recorded WebSocket commands over 20 selections (10 Hotline, 10 ACP): models.efforts 10→1, backends.list 20→1, voice.status 20→1, capabilities.options 20→1; computer.status 20→20. Five UI regressions cover immediate cached labels, settings/provider changes, model keys, remote/endpoint isolation, stale replies, reconnects and refusal retry. dispatch/epic-5 merge reports already up to date (dfb4048d); foreground make check is running.
- 2026-10-21T11:06:13.005Z: [codex] Ready for review on dispatch/task-50, commit 509cf5a7. Desk-connection-scoped metadata cache coalesces pending reads and keeps picker labels stable; room changes/reconnects invalidate it, old replies cannot restore it, and persona computer status stays uncached. Same 20-selection fixture: metadata requests 70→4, computer.status 20→20. Merged dispatch/epic-5 (already current at dfb4048d); final foreground make check passed, and make precheck passed all 8 legs including Windows and isolated UI tests. Next action: reviewer inspect and merge this branch; do not mark complete until reviewed.
