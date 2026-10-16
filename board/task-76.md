---
id: task-76
title: "ADR 0005: the live voice is a full agent on the call"
column: todo
position: 3
priority: high
tags:
  - voice
  - front
  - adr
parentId: epic-7
contract:
  status: ready
  deliverables:
    - type: file
      path: docs/adr/0005-front-is-a-full-agent.md
      description: The decision
    - type: file
      path: docs/voice-calls.md
      description: Calls doc matches
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:03:19.674Z"
createdAt: "2026-10-16T14:03:19.681Z"
---

## Description
Supersede the relay-only rule of ADR 0004 (docs/adr/0004-*.md) with the epic's direction: the front answers on the call, runs the teammate's desk-side tools itself, hands only heavy or code-changing work to the working session, and the chat gets summaries, not per-turn mirrors. Record the provider limit (GPT-Live delegates only, no function tools) and the open question of harness-native tools. Update docs/voice-calls.md and docs/voice.md to match.
