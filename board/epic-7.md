---
id: epic-7
title: "The live voice is the teammate: a full agent on the call, with the chat as its record"
type: epic
column: todo
position: 0
priority: high
tags:
  - voice
  - calls
  - front
  - 0.39.0
createdAt: "2026-10-16T13:52:14.099Z"
landsOn: release/0.39.0
updatedAt: "2026-10-16T14:04:44.290Z"
assignee: claude
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-16T14:04:30.973Z"
  until: review
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T14:04:32.647Z"
---

## Description
George 2026-10-16 08:51, after testing Grok Voice and GPT-Live from the car: routing every utterance through the working session adds a round trip each time and fills the DM ("No, stop" came back as two "Stopped" messages). The value of a live model is that it runs tools and thinks in the voice modality. Direction: the front is a full agent with settings parity with its home teammate, answers on the call itself, and posts a summary of what was done to the main chat at the end of the call or at intervals, so the two read as one teammate.

Shape:
1. Tool parity: the front gets the teammate's grants run desk-side (its MCP servers, skills, web search, the conversation's search/chapters/files, schedules, its computer within its reach and mode), with the same permission asks and audit as a typed turn.
2. Heavy work still has a path: long or code-changing work goes to the working session as today, but by the front's choice, not for every utterance.
3. The call keeps its own transcript; nothing mirrors into the DM per turn. A summary posts at hang-up and every N minutes on a long call; "put that in the chat" posts at once.
4. Settings parity: model, voice, reach and grants follow the teammate's card.
5. Providers: Grok Voice and GPT Realtime take function tools; GPT-Live only delegates, so on GPT-Live the front stays delegate-only (say so in Live calls).

Open questions: how a harness teammate's own tools (Claude Code edits, Codex) map, since those live inside the harness; whether the summary is the front's or the teammate's words; budget for long tool-heavy calls on per-minute keys.

Step 0 shipped separately: 0.38.1 (release/0.38.1, e2fab793) keeps talk about the call in the voice.

## Log
- 2026-10-16T13:57:42.600Z: [claude] 2026-10-16: George chose to roll the small-talk fix into 0.39 rather than ship 0.38.1. Integration branch release/0.39.0 cut from main (desktop-v0.38.0) with e2fab793 on it; 0.39 features merge there.
