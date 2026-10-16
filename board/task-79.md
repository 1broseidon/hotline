---
id: task-79
title: "Desk: GPT-Live runs the teammate's tools through a desk-side tool loop"
column: todo
position: 5
description: "The epic says all three fronts become full agents; GPT-Live has no realtime function tools and delegates tool use to a backend (see ADR 0005, task-76: delegation type responses with a backend model, or client delegation on the ChatGPT sign-in). Give GPT-Live the same registry task-74 builds in voice/front_tools.rs: when the backend returns function calls, run them desk-side through the same handler, grants and audit, and return the outputs. Reuse front_tools; add no second registry. Harness in crates/hotline-core/tests/voice.rs on the GPT-Live dialect fake: a lookup is answered from search_thread with no send_to_teammate, and a tool outside Reach is refused. If the ADR finds the ChatGPT-sign-in path cannot be done without a larger change, write that in a note and stop at the backend-model path."
priority: high
tags:
  - voice
  - front
  - tools
relatedFiles:
  - crates/hotline-core/src/voice/front_tools.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/tests/voice.rs
  - docs/adr/0005-front-is-a-full-agent.md
parentId: epic-7
dependsOn:
  - task-74
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front_tools.rs
      description: GPT-Live tool loop reuses the registry
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: GPT-Live answers from a tool, denied grant refused
  validation:
    commands:
      - make check
      - cargo test -p hotline-core --test voice
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts
    - "Capability changes follow docs/security.md: allowed and denied through the real handler, revocable via the session lease"
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T15:06:10.385Z"
createdAt: "2026-10-16T14:11:07.271Z"
updatedAt: "2026-10-16T15:06:12.950Z"
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-16T15:06:11.476Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T15:06:12.950Z"
assignee: claude
---

## Description
The epic says all three fronts become full agents; GPT-Live has no realtime function tools and delegates tool use to a backend (see ADR 0005, task-76: delegation type responses with a backend model, or client delegation on the ChatGPT sign-in). Give GPT-Live the same registry task-74 builds in voice/front_tools.rs: when the backend returns function calls, run them desk-side through the same handler, grants and audit, and return the outputs. Reuse front_tools; add no second registry. Harness in crates/hotline-core/tests/voice.rs on the GPT-Live dialect fake: a lookup is answered from search_thread with no send_to_teammate, and a tool outside Reach is refused. If the ADR finds the ChatGPT-sign-in path cannot be done without a larger change, write that in a note and stop at the backend-model path.
