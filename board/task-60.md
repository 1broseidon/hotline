---
id: task-60
title: "Desk: front prompt and starting context"
column: todo
position: 40
priority: medium
tags:
  - voice
  - calls
  - 0.38.0
parentId: epic-6
dependsOn:
  - task-59
createdAt: "2026-10-14T20:03:57.384Z"
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/dispatcher.rs
updatedAt: "2026-10-15T01:56:48.978Z"
---

## Description
Front instructions: speaks as the teammate (public name, its voice), relays and clarifies, delegates every request, treats teammate output as authoritative, keeps answers short, says when it is waiting. Starting context within a small budget: the chapter's handoff note and recent lines of the DM, the person's name; no secrets, no tool output dumps. Separate prompt for desk calls (the desk/dispatcher as the teammate behind). Fixture tests for prompt assembly and budgets.
