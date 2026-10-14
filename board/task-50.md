---
id: task-50
title: "Window: cache room-wide picker and inspector lists per desk"
column: backlog
position: 33
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
updatedAt: "2026-10-14T20:05:29.152Z"
---

## Description
Split from task-22, preserving BRO-244. Room-wide model efforts, backends and capability reads are repeated on teammate switches. Share reads within a desk and invalidate on the relevant model/settings changes; never share credentials or data between desks. Keep per-persona computer status fresh. Verify picker stability and changed providers/settings, including remote desk switching; measure request counts on repeated teammate switches.
