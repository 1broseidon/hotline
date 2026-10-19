---
id: epic-20
title: "One agent thread layer: every modality attaches to it, and the chat is the log of truth"
type: epic
column: todo
position: 11
priority: high
tags:
  - architecture
  - threads
  - voice
  - "0.41"
assignee: claude
createdAt: "2026-10-19T17:32:26.752Z"
landsOn: release/0.41.0
updatedAt: "2026-10-19T20:00:43.986Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-19T20:00:43.857Z"
  until: done
  role: lead
---

## Description
George, 2026-10-19 12:20-12:31 (phone):
- Agent = thread. A text DM, a side thread, a subagent, a scheduled or quiet run, a pair thread, a voice call: each is the same agent thread with a different modality attached. One central agent-thread layer that any modality attaches to, rather than a separate path per feature.
- Voice is a modality, not a separate agent: on a call the teammate itself is the full agent, with its tools, with speech in and speech out (see epic-19 task-213).
- Escalation everywhere. Today only a quiet scheduled run on a Hotline Agent can post to the person, through the Escalate trait and its tell_person tool (driver.rs, BRO-96, one message per run). Adopt it, renamed if a better name fits, as the one way any thread posts to the main chat, on every driver and every modality.
- The person always falls back to the chat as the log of truth, and the system is built that way. On a call, a doc, a link, a file or a summary the agent produces is sent to the chat through that primitive, not read out.

Where the code is today: ThreadId already spans the DM, side threads, runs, calls and pair threads (hotline-contract), and session/threads.rs writes them; but each kind still has its own session path, the voice fronts are separate agents (ADR 0005), and escalation exists only for quiet runs.

First the architecture, for George to approve; then the slices, planned from it.

## Log
- 2026-10-19T19:59:30.117Z: [claude] George 14:58: build ADR 0008 in two releases. 0.41 = waves 1 and 2 (the chat door, pairs, artifacts, jobs as scheduled clones, clone permissions; plus epic-17's team lead core and UI), then Pork Chop's own QA on the Linux QA computer, then George QA's the desk. 0.42 = the voice path (epic-21). Lead: only this epic's children land on release/0.41.0.
