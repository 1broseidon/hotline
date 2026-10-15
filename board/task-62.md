---
id: task-62
title: "Desk: record front calls truthfully and count their cost"
priority: medium
tags:
  - voice
  - calls
  - ledger
  - 0.38.0
parentId: epic-6
dependsOn:
  - task-59
  - task-61
createdAt: "2026-10-14T20:03:58.314Z"
relatedFiles:
  - crates/hotline-core/src/voice/record.rs
  - crates/hotline-core/src/voice/ledger.rs
  - crates/hotline-core/src/voice/metering.rs
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/tests/voice.rs
  - docs/voice-calls.md
updatedAt: "2026-10-15T17:08:41.814Z"
contract:
  status: done
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/record.rs
      description: Front transcript to calls/<callId>.jsonl and DM marker
    - type: file
      path: crates/hotline-core/src/voice/ledger.rs
      description: Per-minute front cost, zero for subscription logins, budget check
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: Record kept apart from tape; cost counted by login kind
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy -p hotline-core --all-targets -- -D warnings
      - cargo test -p hotline-core
  constraints:
    - Teammate tape holds only what was handed to it and what it answered
    - Run unfiltered; regenerate contract.ts if VoiceStatus changes
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
    pickedUpAt: "2026-10-15T02:40:25.602Z"
    reworkCount: 0
    deliveredAt: "2026-10-15T02:46:42.331Z"
    duration: 377
assignee: claude
dispatch:
  agent: claude
  model: sonnet
  effort: medium
  by: lead
  at: "2026-10-15T02:40:16.216Z"
column: review
position: 1
---

## Description
Front transcript (person and front lines) goes to calls/<callId>.jsonl and the DM call marker, kept apart from the teammate's tape, which holds only what was handed to it and what it answered. Ledger: per-minute cost for API-key sessions (Grok $0.08/min, GPT-Live $0.05/min), zero for subscription logins, shown in voice status; budget checks before a call starts. Note subscription limits once known.

## Log
- 2026-10-15T02:46:42.051Z: [claude] Added Kind::Front and ledger::front_usd (xai $0.08/min, openai $0.05/min, subscription/local free, unknown provider charged at $0.08), counted in the Voice budget so voice.status budget shows it with no contract change. Calls gained front_ready (budget check before a call starts), front_spend(front, seconds) (charge; Err means end the call), front_heard / front_said (front lines to calls/<id>.jsonl; the teammate tape is untouched). For later tasks: nothing yet runs a RealtimeSession, so whoever wires it must call front_ready before opening, front_spend about once a minute, and front_heard/front_said from RealtimeEvent::Heard/Saying (said lines are kind "agent" on the call thread). Subscription limits are not yet known and not noted. Tests in tests/voice.rs; cargo test -p hotline-core, clippy and fmt pass. On branch dispatch/task-62.
- 2026-10-15T17:07:58.894Z: Reopened into In progress
