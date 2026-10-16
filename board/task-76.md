---
id: task-76
title: "ADR 0005: the live voice is a full agent on the call"
column: in-progress
position: 3
priority: high
tags:
  - voice
  - front
  - adr
parentId: epic-7
contract:
  status: in_progress
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
    pickedUpAt: "2026-10-16T14:06:10.812Z"
    reworkCount: 0
createdAt: "2026-10-16T14:03:19.681Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-16T14:05:46.777Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T14:05:48.659Z"
assignee: claude
updatedAt: "2026-10-16T14:06:10.812Z"
---

## Description
Supersede the relay-only rule of ADR 0004 (docs/adr/0004-*.md) with the epic's direction: the front answers on the call, runs the teammate's desk-side tools itself, hands only heavy or code-changing work to the working session, and the chat gets summaries, not per-turn mirrors. Record the provider limit (GPT-Live delegates only, no function tools) and the open question of harness-native tools. Update docs/voice-calls.md and docs/voice.md to match.

## Log
- 2026-10-16T14:06:29.196Z: [claude] CORRECTION — GPT-Live does function calling (George, 2026-10-16; developers.openai.com gpt-live-1 lists streaming and function_calling): it delegates reasoning and tool use to a backend, and the backend's function calls come back to the client as response events, answered with function_call_output. Two ways to serve it, to settle in task-74: (a) delegation type responses, a backend model with our desk-side tools declared as functions, each call run desk-side; (b) the ChatGPT sign-in route we use today, delegation type client, answered by a desk-side tool loop instead of the working session. Whether the sign-in route accepts responses delegation is unverified: spike it. All three fronts (Grok Voice, GPT Realtime, GPT-Live) get the full agent; none stays delegate-only.
