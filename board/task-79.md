---
id: task-79
title: "Desk: GPT-Live runs the teammate's tools through a desk-side tool loop"
column: review
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
  status: delivered
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
    pickedUpAt: "2026-10-16T15:06:36.952Z"
    reworkCount: 0
    deliveredAt: "2026-10-16T15:17:26.326Z"
    duration: 649
createdAt: "2026-10-16T14:11:07.271Z"
updatedAt: "2026-10-16T15:17:31.024Z"
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
    ended: "2026-10-16T15:17:31.024Z"
    outcome: review
assignee: claude
---

## Description
The epic says all three fronts become full agents; GPT-Live has no realtime function tools and delegates tool use to a backend (see ADR 0005, task-76: delegation type responses with a backend model, or client delegation on the ChatGPT sign-in). Give GPT-Live the same registry task-74 builds in voice/front_tools.rs: when the backend returns function calls, run them desk-side through the same handler, grants and audit, and return the outputs. Reuse front_tools; add no second registry. Harness in crates/hotline-core/tests/voice.rs on the GPT-Live dialect fake: a lookup is answered from search_thread with no send_to_teammate, and a tool outside Reach is refused. If the ADR finds the ChatGPT-sign-in path cannot be done without a larger change, write that in a note and stop at the backend-model path.

## Log
- 2026-10-16T15:17:26.024Z: [claude] Done on dispatch/task-79, make check green. GPT-Live (realtime/live.rs): a teammate call (config.tools has any desk tool beyond send_to_teammate/remain_silent) now creates the call with delegation type responses, BACKEND_MODEL ("gpt-6-luna", a placeholder from OpenAI's example) and the whole registry as function tools; a desk call keeps type client. The backend's response.event/response.output_item.done function_call becomes RealtimeEvent::ToolCall, which duplex.rs already routes to FrontTools (same handler, lease, clip, no DM post); ToolResult goes back as response.item.create function_call_output then response.create. No second registry; front_tools.rs needed no change. Harness tests/voice.rs: lookup answered from search_thread with no hand-off, send_file refused; unit tests for the delegation shape and event mapping. UNVERIFIED: no ChatGPT login to spike, so whether the sign-in accepts responses delegation, that model, the tools shape inside delegation.responses, and whether function_call_output needs delegation_id are from the public guide only; a rejected call falls back to the pipeline. The client-delegation desk-run tool loop is NOT built (needs a desk-run model and budget). Left for task-77/78: run the ignored live test with a real login to confirm; front_prompt.rs does not mention the tools; the instructions suffix about delegating is dropped in responses mode.
