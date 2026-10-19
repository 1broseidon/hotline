---
id: task-223
title: "Slice 5: a call has its own agent and thread; continue as text, call on any thread, summary 30 s after hang-up"
column: backlog
position: 36
priority: high
tags:
  - voice
  - threads
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
  - crates/hotline-core/src/session
parentId: epic-21
createdAt: "2026-10-19T20:00:06.937Z"
---

## Description
ADR 0008 §5 and §6 slice 5, with George's decisions: the Call row as a thread whose agent is the teammate; Legacy voice feeds the call thread instead of the DM; hang-up detaches; the closing note is posted 30 s after hang-up once no turn is in flight and the thread parks, still continuable; a call continues as text and a new call can pick it up; a call can start on any thread under way. Fronts still run as today when chosen. Removes what slice 5 lists.
