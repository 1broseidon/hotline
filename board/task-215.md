---
id: task-215
title: "ADR: the agent thread layer, modalities, and one escalation path to the chat"
column: todo
position: 12
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
updatedAt: "2026-10-19T17:32:32.097Z"
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
