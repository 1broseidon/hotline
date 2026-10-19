---
id: task-201
title: "Design review: one voice front, the team lead role, routing and acknowledgements"
column: todo
position: 1
priority: high
tags:
  - design
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/front_tools.rs
  - docs/adr
parentId: epic-17
createdAt: "2026-10-19T13:59:37.216Z"
---

## Description
A short ADR (next free number) for George before code moves: how desk and teammate calls become one front path (one rules builder; the only difference is who speaks), what the lead's front is given (its own context and tools, plus the roster and routing), when the dispatcher model is still used (only when it is unclear who owns a request, or with no lead), how the lead hands work to a named teammate directly, what happens when the lead is busy (proposal: the desk call opens as a side thread with the lead, its main conversation untouched), the lead's extra grant (hand off to teammates on a desk call without first-contact approval; stopping or reassigning others stays the person's), the acknowledgement design (generated from the request, recent ones fed back, earcon on a silent hand-off) and how the "never guess" rule is enforced. Docs only; stops in Review for George.
