---
id: task-236
title: "S6: voice metering keyed by dispatch id (Rig 0.45's exactly-once outcome hook)"
column: todo
position: 8
priority: medium
tags:
  - rig
  - voice
  - budgets
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:45.659Z"
---

## Description
Report §6 S6 and §5. One hook: on_dispatch reserves (deny when over budget), on_outcome settles by EffectId; remove the single reservation slot. Tests: a provider error keeps its reservation; a retried turn settles both attempts; a stream dropped mid-reply stays charged; a budget refusal lands on the next dispatch.
