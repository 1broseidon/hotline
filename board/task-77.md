---
id: task-77
title: "Desk: the front prompt is a full agent's, and settings follow the teammate"
column: todo
position: 4
priority: medium
tags:
  - voice
  - front
  - settings
  - ui
parentId: epic-7
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front_prompt.rs
      description: Full-agent rules
    - type: file
      path: ui/src/components/UseFor.tsx
      description: GPT-Live's line says it delegates
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:03:20.244Z"
createdAt: "2026-10-16T14:03:20.251Z"
dependsOn:
  - task-74
  - task-75
updatedAt: "2026-10-16T14:05:22.988Z"
---

## Description
Rewrite the front prompts (voice/front_prompt.rs, TEAMMATE_RULES and DESK_RULES) for a full agent: answer conversation itself, use tools for lookups, hand heavy or code-changing work to the working session with one short acknowledgement, never fill silence. The tools it has are listed to it from front_tools. Settings parity: the front's voice follows the teammate's card (already), and its grants follow the teammate's Reach (MCP servers, computer within mode). Live calls in Settings (ui/src/components/UseFor.tsx) says GPT-Live 'answers by handing your words to the teammate' since it has no tools. Prompt tests pin the new rules.
