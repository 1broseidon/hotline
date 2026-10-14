---
id: task-57
title: "Voice front: decision record and call docs"
column: todo
position: 37
priority: high
tags:
  - voice
  - calls
  - docs
  - 0.37.0
parentId: epic-6
createdAt: "2026-10-14T20:03:55.988Z"
---

## Description
ADR in docs/adr: live calls get a realtime voice front; supersedes 'No front' in docs/voice-calls.md (Direct turns). Record why (latency, barge-in, subscriptions cover it), the Codex precedent, what stays (teammate does the work in its own session; old pipeline as fallback) and the guardrails (front relays and clarifies, never claims work; client picks spoken text). Update voice-calls.md wire ledger and voice.md providers as the children land.
