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
  - 0.38.0
parentId: epic-6
createdAt: "2026-10-14T20:03:55.988Z"
relatedFiles:
  - docs/adr/0003-live-calls-get-a-voice-front.md
  - docs/voice-calls.md
  - docs/voice.md
  - docs/design.md
updatedAt: "2026-10-15T01:57:04.230Z"
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/adr/0003-live-calls-get-a-voice-front.md
      description: "ADR: why a realtime voice front, Codex precedent, what stays, guardrails"
    - type: docs
      path: docs/voice-calls.md
      description: "'No front' in Direct turns replaced by the front design"
  validation:
    commands:
      - test -f docs/adr/0003-live-calls-get-a-voice-front.md
      - grep -qi 'voice front' docs/voice-calls.md
      - "! grep -q 'Direct turns | No front' docs/voice-calls.md"
      - python3 -m unittest discover -s scripts -p 'test_*.py'
  constraints:
    - Docs only; no code changes
    - Say what is decided, not what is built yet; docs/ holds only what is, so mark unlanded parts as planned for 0.38
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
---

## Description
ADR in docs/adr: live calls get a realtime voice front; supersedes 'No front' in docs/voice-calls.md (Direct turns). Record why (latency, barge-in, subscriptions cover it), the Codex precedent, what stays (teammate does the work in its own session; old pipeline as fallback) and the guardrails (front relays and clarifies, never claims work; client picks spoken text). Update voice-calls.md wire ledger and voice.md providers as the children land.
