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
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/tests/failures.rs
parentId: epic-22
createdAt: "2026-10-20T06:51:45.659Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T12:03:22.957Z"
---

## Description
Report §6 S6 and §5 (docs/plans/rig-0.45-migration.md). Builds on S1's minimal voice port in `voice/dispatcher.rs`. Replace `Meter`/`StreamMeter` with one hook: `on_dispatch` reserves (deny when over budget), `on_outcome` settles by `EffectId`; remove the single reservation slot. Record the metering rule in docs/voice-calls.md or docs/design.md, wherever voice budgets are described today.

Tests, prefixed `voice_meter_`: a provider error keeps its reservation; a retried turn settles both attempts; a stream dropped mid-reply stays charged; a budget refusal lands on the next dispatch. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).
