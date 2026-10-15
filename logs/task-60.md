---
id: task-60
title: "Desk: front prompt and starting context"
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
updatedAt: "2026-10-15T02:28:48.146Z"
contract:
  status: failed
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front_prompt.rs
      description: Front instructions and starting context assembly
    - type: test
      path: crates/hotline-core/src/voice/front_prompt.rs
      description: Fixture tests for prompt assembly and budgets
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy -p hotline-core --all-targets -- -D warnings
      - cargo test -p hotline-core
  constraints:
    - Budgeted context; no secrets or tool-output dumps
    - Separate desk-call prompt
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
    pickedUpAt: "2026-10-15T02:23:03.571Z"
    reworkCount: 0
    deliveredAt: "2026-10-15T02:28:44.020Z"
    duration: 340
  feedback: "crates/hotline-core/src/voice/front_prompt.rs (/Users/george/Projects/personal/hotline/crates/hotline-core/src/voice/front_prompt.rs): File not found"
assignee: claude
dispatch:
  agent: claude
  model: sonnet
  effort: medium
  by: lead
  at: "2026-10-15T02:22:50.604Z"
completedAt: "2026-10-15T02:28:48.146Z"
---

## Description
Front instructions: speaks as the teammate (public name, its voice), relays and clarifies, delegates every request, treats teammate output as authoritative, keeps answers short, says when it is waiting. Starting context within a small budget: the chapter's handoff note and recent lines of the DM, the person's name; no secrets, no tool output dumps. Separate prompt for desk calls (the desk/dispatcher as the teammate behind). Fixture tests for prompt assembly and budgets.

## Log
- 2026-10-15T02:28:43.755Z: [claude] Added voice/front_prompt.rs on branch dispatch/task-60: pure `instructions(&Behind, &Start) -> String` (Behind::Teammate{public_name, voice} | Desk; Start{person, handoff, recent: &[Line]}). Budgets: name 60, handoff 600, voice 600, 6 DM lines ≤240 chars each ≤1200 total (newest kept); text scrubbed via driver::failure::redact_text (now pub(crate)) and code fences dropped. Next tasks: caller must pass only spoken DM lines (no tool output) and set RealtimeConfig.instructions from this; not yet wired to any call. fmt, clippy, cargo test -p hotline-core pass.
- 2026-10-15T02:28:47.864Z: [claude] Contract validate resolved paths against the main checkout, so the file deliverable reads "not found" until this branch merges; the file exists at crates/hotline-core/src/voice/front_prompt.rs on dispatch/task-60, committed.
