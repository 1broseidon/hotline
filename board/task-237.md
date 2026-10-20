---
id: task-237
title: "S7: cleanups 0.45 makes possible"
column: todo
position: 9
priority: low
tags:
  - rig
assignee: claude
relatedFiles:
  - crates/hotline-core/src/providers/copilot.rs
  - crates/hotline-core/src/providers/compat.rs
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/rig/turn.rs
parentId: epic-22
createdAt: "2026-10-20T06:51:46.152Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T12:03:23.498Z"
---

## Description
Report §6 S7 (docs/plans/rig-0.45-migration.md). Cleanups 0.45 makes possible, each proved by S0's `rig_pin_` snapshots staying unchanged: delete Copilot's `ChatBody` if the Copilot string-content pin passes without it; Copilot `/responses` via `with_route(Route::Responses)` on the shared dialect; `compat.rs` probe on `Value`. Optional, only if it makes the code smaller and the pins stay green: typed `.reasoning(Effort)` with `OnUnsupported::Ignore` replacing `effort_params` (check Gemini's `thinkingLevel` casing, report §7.9); a `pairing::validate` debug assert before each request. Say in your final note which optional items you took and why. Do not touch `voice/dispatcher.rs` (S6) or `providers/rig_login.rs` (S4); do not edit docs/plans/rig-0.45-migration.md (the closing task does).
