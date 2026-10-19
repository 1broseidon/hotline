---
id: task-216
title: "Slice 1: Room::post, one door to the chat, posts stamped with where they came from"
column: todo
position: 12
priority: high
tags:
  - threads
  - chat
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-contract
  - docs/adr/0008-one-agent-thread-layer.md
parentId: epic-20
createdAt: "2026-10-19T19:59:30.596Z"
---

## Description
ADR 0008 §3 and §6 slice 1, as written there: Room::post(teammate, from, post); call_posted, the quiet run's follow-up and send_file from a thread write through it; TranscriptEvent::Agent gains optional `from` and `scheduled` (the reference importer reads them; design.md says so); a quiet run's note is posted as written; posts reach the home agent by who asked (§3). Removes what slice 1 lists. Proved by the headless harness the ADR names.
