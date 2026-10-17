---
id: task-51
title: "ACP: show a subagent working again when it resumes after completion"
column: backlog
position: 34
description: "Split from task-36's independent lifecycle bug. Observed 2026-10-09: Claude Code child finishes its turn with a background job still running; Hotline marks the run done, and later resumed work stays invisible. Investigate actual child-session updates after subagent_state_update: completed; do not assume the adapter supplies a new spawn announcement. If updates arrive, restore working state and stream subsequent steps without duplicate runs; if they do not, record the adapter limitation. Prove completed → resumed → completed with a scripted ACP child and the real core wire. The work-view redesign in task-36 does not block this fix."
priority: high
tags:
  - bug
  - acp
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - ui/src/components/Dock.tsx
createdAt: "2026-10-12T04:20:55.329Z"
updatedAt: "2026-10-17T05:12:07.048Z"
parentId: epic-9
---

## Description
Split from task-36's independent lifecycle bug. Observed 2026-10-09: Claude Code child finishes its turn with a background job still running; Hotline marks the run done, and later resumed work stays invisible. Investigate actual child-session updates after subagent_state_update: completed; do not assume the adapter supplies a new spawn announcement. If updates arrive, restore working state and stream subsequent steps without duplicate runs; if they do not, record the adapter limitation. Prove completed → resumed → completed with a scripted ACP child and the real core wire. The work-view redesign in task-36 does not block this fix.
