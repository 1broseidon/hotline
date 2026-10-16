---
id: task-78
title: "Desk: one call proves the whole front — tools, hand-off, summary, settings — and the docs say so"
column: todo
position: 5
priority: high
tags:
  - voice
  - front
  - integration
relatedFiles:
  - crates/hotline-core/tests/voice.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/front_tools.rs
  - docs/voice-calls.md
  - docs/adr/0005-front-is-a-full-agent.md
parentId: epic-7
dependsOn:
  - task-74
  - task-75
  - task-76
  - task-77
  - task-79
  - task-80
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: One call proves tools, hand-off, post, summary, denial and settings; GPT-Live stays delegate-only
    - type: docs
      path: docs/voice-calls.md
      description: Matches what shipped
  validation:
    commands:
      - make check
      - cargo test -p hotline-core --test voice
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:05:40.647Z"
createdAt: "2026-10-16T14:05:40.652Z"
updatedAt: "2026-10-16T15:42:20.773Z"
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-16T15:42:20.631Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Wire and prove the epic end to end against its description of done. One headless harness in crates/hotline-core/tests/voice.rs drives a single call over the wire on a function-tool dialect (Grok Voice / GPT Realtime socket fake): the front answers a lookup from a desk-side tool (no send_to_teammate), hands one code-changing request to the working session with one short acknowledgement, honours 'put that in the chat' via post_to_chat, and at hang-up the DM holds exactly the hand-off's turn, the post, and one first-person summary (task-80: the teammate's own words, no tool names, also in the teammate's history so a typed follow-up carries on from the call); a teammate with a tool not in its Reach has it refused; the front's voice and model follow the teammate's card. A second case on the GPT-Live dialect (task-79) shows the same lookup answered through its desk-side tool loop and a denied tool refused, with heavy work still handed off. Fix whatever gaps between the children this exposes. Check docs/voice-calls.md, docs/voice.md, docs-site/src/content/docs/use/voice.md and ADR 0005 against what shipped, and record any still-open question (harness-native tools, per-minute budget; summary voice if task-80 left it open) in the ADR.
