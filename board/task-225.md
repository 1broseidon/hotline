---
id: task-225
title: "Slice 6: the live front's real work goes to a clone through a first-person work tool; no hand-off talk"
column: backlog
position: 38
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-21
createdAt: "2026-10-19T20:00:07.934Z"
---

## Description
ADR 0008 §5 and §6 slice 6: the live front keeps its voice and lookups; one work tool runs a clone of the teammate on its own model and harness, focused on the task, on the call's thread, returning at once and reporting back into the call; artifacts post to the chat; the home agent is never interrupted; GPT-Live's delegation answered by the same tool. Never sounding like a hand-off: first-person tool framing, no stock acknowledgements, recent acknowledgements fed back, and a relay-phrasing regression check in the scripted-front tests. Folds in task-204 (acknowledgements) and task-207 (the chat pointer only for real artifacts, no repeated 'details are in the chat'). Removes what slice 6 lists; ADR 0005 marked amended.
