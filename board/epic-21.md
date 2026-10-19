---
id: epic-21
title: "Voice on the agent thread layer (0.42): a call has its own agent, the front's work goes to a clone, and desk calls reach the lead"
type: epic
column: todo
position: 18
priority: high
tags:
  - voice
  - threads
  - "0.42"
assignee: claude
createdAt: "2026-10-19T20:00:05.973Z"
---

## Description
ADR 0008 waves 3 and 4 (George, 2026-10-19 14:58: two releases; 0.41 is the thread layer, jobs and team lead; 0.42 is voice). Lands on release/0.42.0, cut from release/0.41.0 once 0.41 ships; not dispatched before then.

The rule: voice changes how you talk to the teammate, never who does the work (ADR 0008 §5). Live voice: the live model talks and does lookups, and sends real work to a clone on the teammate's own model and harness through a first-person work tool that never sounds like a hand-off. Legacy voice: speech around the teammate itself. A call is a thread: it continues as text, a new call can pick it up, a call can start on any thread, and its closing note is posted 30 seconds after hang-up once no turn is in flight.

Order: slice 5, then George's proving call on the phone, then slice 6, slice 7, the calls' move into hotline-voice, phone parity, docs and end to end. Pork Chop QA's on the Linux QA computer before George, as for 0.41.
