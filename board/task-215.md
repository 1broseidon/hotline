---
id: task-215
title: "ADR: the agent thread layer, modalities, and one escalation path to the chat"
column: review
position: 0
priority: high
tags:
  - design
  - architecture
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-core/src/session/threads.rs
  - crates/hotline-core/src/driver.rs
  - crates/hotline-contract
  - crates/hotline-core/src/voice
  - docs/adr
  - docs/threads.md
  - docs/sessions.md
parentId: epic-20
createdAt: "2026-10-19T17:32:28.780Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-19T17:32:30.960Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T17:32:32.097Z"
updatedAt: "2026-10-19T17:42:13.162Z"
---

## Description
Write the next free ADR: today's map (every thread kind, the session path each takes, who owns its agent, how its words reach the person, and where escalation exists), then the target:
1. The agent thread: what one is (its agent, lease, stream, context), what every modality shares, and the single path that drives it.
2. A modality: text DM, side thread, subagent, scheduled/quiet run, pair thread, voice call. What each attaches (input, output, whether its words are visible, whether it may wake the person), and what stays specific to it.
3. Escalation: one primitive (keep the name or propose a better one) every thread on every driver can use to post to the main chat, with the safeguards today's has (who may, how often, how it's stamped and notified), and an equivalent for ACP agents (an MCP tool on the desk's server).
4. The chat as log of truth: what always reaches it (a call's summary, any doc, link or file an agent produces in any modality, an escalation) and what never does (a quiet run's thinking, a call's small talk).
5. Voice under this design: speech in and out around the full agent; how it relates to epic-19's crate move and task-213's interface; what happens to the live fronts.
6. Migration in slices, each landable on its own, with what each removes.
Stops in Review for George; no code.

## Log
- 2026-10-19T17:42:12.367Z: [claude] Wrote docs/adr/0008-one-agent-thread-layer.md (proposed; indexed in design.md), on branch dispatch/task-215 at d5de756e, docs only. It covers today's map with citations at 419bdac5, then the target: the agent thread, a modality table, post_to_chat as the one escalation door on the MCP server for both drivers (Room::post, once per turn, lease-checked, stamped from:, runs excluded), the chat as log of truth, and voice as a call thread whose agent is the teammate. It recommends running calls on the Speech engines without the fronts, and says task-213 should size its interface against this target and task-214 should move the calls after slice 6. Seven migration slices, each listing what it removes. Found along the way: driver.rs Escalate doc and mcp/server.rs "Sixteen tools" are stale, and an ACP quiet run cannot escalate today.
