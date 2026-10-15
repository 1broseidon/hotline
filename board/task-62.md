---
id: task-62
title: "Desk: record front calls truthfully and count their cost"
column: todo
position: 42
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
updatedAt: "2026-10-15T01:57:04.230Z"
contract:
  status: ready
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
---

## Description
Front transcript (person and front lines) goes to calls/<callId>.jsonl and the DM call marker, kept apart from the teammate's tape, which holds only what was handed to it and what it answered. Ledger: per-minute cost for API-key sessions (Grok $0.08/min, GPT-Live $0.05/min), zero for subscription logins, shown in voice status; budget checks before a call starts. Note subscription limits once known.
