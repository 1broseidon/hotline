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
    pickedUpAt: "2026-10-16T14:11:48.865Z"
    reworkCount: 1
createdAt: "2026-10-16T14:03:19.681Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-16T14:11:27.976Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T14:11:30.214Z"
assignee: claude
updatedAt: "2026-10-16T14:11:48.865Z"
---

## Description
Supersede the relay-only rule of ADR 0004 (docs/adr/0004-*.md): the front answers on the call, runs the teammate's desk-side tools itself, hands only heavy or code-changing work to the working session, and the chat gets summaries, not per-turn mirrors. Tools by provider: Grok Voice and GPT Realtime take function tools on the realtime session; GPT-Live delegates tool use to a backend whose function calls come back to the desk to run (delegation type responses with a backend model, or client delegation on the ChatGPT sign-in answered by a desk-side tool loop). None of the three stays delegate-only. Record the open question of harness-native tools. Update docs/voice-calls.md and docs/voice.md to match.

## Log
- 2026-10-16T14:06:29.196Z: [claude] CORRECTION — GPT-Live does function calling (George, 2026-10-16; developers.openai.com gpt-live-1 lists streaming and function_calling): it delegates reasoning and tool use to a backend, and the backend's function calls come back to the client as response events, answered with function_call_output. Two ways to serve it, to settle in task-74: (a) delegation type responses, a backend model with our desk-side tools declared as functions, each call run desk-side; (b) the ChatGPT sign-in route we use today, delegation type client, answered by a desk-side tool loop instead of the working session. Whether the sign-in route accepts responses delegation is unverified: spike it. All three fronts (Grok Voice, GPT Realtime, GPT-Live) get the full agent; none stays delegate-only.
- 2026-10-16T14:11:15.742Z: [claude] Lead: last hand-off was stopped (no run in flight). Re-dispatching. ADR must say all three fronts are full agents: Grok/GPT Realtime via session function tools (task-74), GPT-Live via a desk-side tool loop over its backend (task-79), not delegate-only. Record harness-native tools as open.
- 2026-10-16T14:17:03.068Z: [claude] Done on branch dispatch/task-76 (b28814a8, make check green): ADR 0005 written. Front is a full agent on direct calls; "Tools by provider" says Grok Voice/GPT Realtime get the registry in session.update (task-74), GPT-Live answers its delegations with function_call_output from a desk-side tool loop (task-79: spike whether the ChatGPT sign-in accepts delegation type responses, else client delegation); none delegate-only. Open questions: harness-native tools, summary authorship, call-level tool budget, GPT-Live loop model/budget. ADR 0004 status marked superseded in part. voice-calls.md Direct turns and voice.md (One brain; gpt-live paragraph) still describe the relay that runs and point to 0005 — each epic-7 task rewrites its part as it lands; task-78 removes the pointers.
