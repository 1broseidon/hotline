---
id: task-203
title: "Voice: calling the desk reaches the team lead through the one front path"
column: todo
position: 3
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/front_tools.rs
parentId: epic-17
createdAt: "2026-10-19T13:59:38.168Z"
dependsOn:
  - task-201
  - task-202
updatedAt: "2026-10-19T13:59:51.090Z"
---

## Description
Per the design review: one rules builder for both calls; a desk call with a lead runs the lead's front with the lead's context and lookup tools plus the roster and a way to hand words straight to a named teammate; the dispatcher model only where the review keeps it. With no lead, today's desk call, but given the person's name and the roster instead of Start::default(). Harness tests over the scripted front: a desk call with a lead answers a roster question itself, hands a named teammate's work directly, and never routes through the dispatcher when the owner is clear.
