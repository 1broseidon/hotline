---
id: task-74
title: "Desk: the front runs the teammate's desk-side tools itself"
column: review
position: 1
priority: high
tags:
  - voice
  - front
  - tools
parentId: epic-7
contract:
  status: delivered
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front_tools.rs
      description: Tool registry and desk-side execution
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: A front call answers from a tool with no hand-off, and a denied grant is refused through the real handler
  validation:
    commands:
      - make check
      - cargo test -p hotline-core --test voice
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - "Capability changes follow docs/security.md: prove each tool allowed and denied through the real handler, revocable via the session lease, and state default, grant source, enforcement points, tests, residual risk in the PR"
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:05:40.080Z"
    pickedUpAt: "2026-10-16T14:23:33.153Z"
    reworkCount: 0
    deliveredAt: "2026-10-16T14:42:55.647Z"
    duration: 1162
createdAt: "2026-10-16T14:02:49.220Z"
dependsOn:
  - task-76
updatedAt: "2026-10-16T14:43:03.967Z"
relatedFiles:
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/front_tools.rs
  - crates/hotline-core/tests/voice.rs
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-16T14:23:11.521Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T14:23:13.457Z"
    ended: "2026-10-16T14:43:03.967Z"
    outcome: review
assignee: claude
---

## Description
Give the voice front the teammate's own tools as realtime function tools, run desk-side with the same permission asks, grants and audit as a typed turn: search_thread, list_chapters, read a chapter, list files and links, list schedules, web_search, and the MCP servers the teammate is granted (Reach). Grok Voice and GPT Realtime take them through the socket driver's session.update tools (voice/realtime/socket.rs, both dialects). GPT-Live has no realtime function tools; it is task-79's, which reuses the registry built here, so keep the registry provider-neutral. A tool result goes back to the model as the function output; nothing is posted to the DM. New module voice/front_tools.rs; wire in voice/front.rs (the bridge) and voice/duplex.rs. Budget: a tool call's output is clipped before the model hears it. Harness: a front call that asks 'what did we work on last week' answers from search_thread without a send_to_teammate.

## Log
- 2026-10-16T14:06:29.802Z: [claude] CORRECTION — GPT-Live does function calling (George, 2026-10-16; developers.openai.com gpt-live-1 lists streaming and function_calling): it delegates reasoning and tool use to a backend, and the backend's function calls come back to the client as response events, answered with function_call_output. Two ways to serve it, to settle in task-74: (a) delegation type responses, a backend model with our desk-side tools declared as functions, each call run desk-side; (b) the ChatGPT sign-in route we use today, delegation type client, answered by a desk-side tool loop instead of the working session. Whether the sign-in route accepts responses delegation is unverified: spike it. All three fronts (Grok Voice, GPT Realtime, GPT-Live) get the full agent; none stays delegate-only.
- 2026-10-16T14:42:55.340Z: [claude] Done on branch dispatch/task-74, make check green. voice/front_tools.rs: provider-neutral registry (FrontTools::open/definitions/has/call) over TeammateTools::for_front (search_thread, list_chapters, list_schedules, web_search; refuses everything else) plus MCP servers the teammate's policy grants (connected at call open, 10s limit), bound to the teammate's current lease via Room::front_reach (revoking on the card refuses the next call); output clipped to 4000 chars; calls run in a spawned task beside the call and answer with reply=true; nothing posted to the DM. Grok/GPT Realtime get them in session.update on direct calls only (desk calls none). Harness in tests/voice.rs: search answers with no send_to_teammate, send_file refused, revoked grant refused. Unwired for later: GPT-Live (task-79: reuse FrontTools::definitions/call as the desk-side loop); 'read a chapter' and 'list files and links' have no existing teammate tool so are not in the registry; front_prompt.rs does not yet tell the model about the tools (task-77); no permission-ask path since these reads and granted MCP tools need none in a typed turn either; harness-native tools still open.
