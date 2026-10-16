---
id: task-77
title: "Desk: the front prompt is a full agent's, and settings follow the teammate"
column: review
position: 4
priority: medium
tags:
  - voice
  - front
  - settings
  - ui
parentId: epic-7
contract:
  status: delivered
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
    pickedUpAt: "2026-10-16T15:31:39.355Z"
    reworkCount: 0
    deliveredAt: "2026-10-16T15:39:44.678Z"
    duration: 485
createdAt: "2026-10-16T14:03:20.251Z"
dependsOn:
  - task-74
  - task-75
  - task-79
  - task-80
updatedAt: "2026-10-16T15:41:24.040Z"
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-16T15:31:02.494Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T15:31:05.266Z"
    ended: "2026-10-16T15:39:55.336Z"
    outcome: review
assignee: claude
---

## Description
Rewrite the front prompts (voice/front_prompt.rs, TEAMMATE_RULES and DESK_RULES) for a full agent: answer conversation itself, use tools for lookups, hand heavy or code-changing work to the working session with one short acknowledgement, never fill silence. The tools it has are listed to it from front_tools. Settings parity: the front's voice follows the teammate's card (already), and its grants follow the teammate's Reach (MCP servers, computer within mode). Live calls in Settings lists all three fronts as full agents. Prompt tests pin the new rules.

## Log
- 2026-10-16T14:06:30.385Z: [claude] CORRECTION — GPT-Live does function calling (George, 2026-10-16; developers.openai.com gpt-live-1 lists streaming and function_calling): it delegates reasoning and tool use to a backend, and the backend's function calls come back to the client as response events, answered with function_call_output. Two ways to serve it, to settle in task-74: (a) delegation type responses, a backend model with our desk-side tools declared as functions, each call run desk-side; (b) the ChatGPT sign-in route we use today, delegation type client, answered by a desk-side tool loop instead of the working session. Whether the sign-in route accepts responses delegation is unverified: spike it. All three fronts (Grok Voice, GPT Realtime, GPT-Live) get the full agent; none stays delegate-only.
- 2026-10-16T15:39:44.377Z: [claude] Done on dispatch/task-77, make check green. front_prompt.rs: both rule sets rewritten as full agents (answer conversation, tools for lookups, hand heavy/changing work on with one ack, never fill silence); Start.tools lists the call's FrontTools::definitions into the prompt ("Your tools, besides send_to_teammate"), wired in duplex.rs (desk calls hold none, so no list). Tests pin the rules and the list. UseFor.tsx: Live calls line says a full agent that looks things up and hands heavy work on; comment says GPT-Live delegates reasoning to a backend. Settings parity needed no change: voice follows the card and grants follow Reach via Room::front_reach. Unwired: desk-call front still has no tools (by design of task-74); GPT-Live's responses-mode instructions suffix about delegating still dropped; no new harness (prompt text only, unit tests); ignored live GPT-Live test still unrun.
- 2026-10-16T15:41:24.040Z: Landed on release/0.39.0
