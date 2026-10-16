---
id: task-74
title: "Desk: the front runs the teammate's desk-side tools itself"
column: todo
position: 1
priority: high
tags:
  - voice
  - front
  - tools
parentId: epic-7
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front_tools.rs
      description: Tool registry and desk-side execution
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: A front call answers from a tool with no hand-off
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:02:49.214Z"
createdAt: "2026-10-16T14:02:49.220Z"
---

## Description
Give the voice front the teammate's own tools as realtime function tools, run desk-side with the same permission asks, grants and audit as a typed turn: search_thread, list_chapters, read a chapter, list files and links, list schedules, web_search, and the MCP servers the teammate is granted (Reach). Grok Voice and GPT Realtime take them through the socket driver's session.update tools (voice/realtime/socket.rs, both dialects); GPT-Live has no function tools and keeps delegate-only. A tool result goes back to the model as the function output; nothing is posted to the DM. New module voice/front_tools.rs; wire in voice/front.rs (the bridge) and voice/duplex.rs. Budget: a tool call's output is clipped before the model hears it. Harness: a front call that asks 'what did we work on last week' answers from search_thread without a send_to_teammate.
