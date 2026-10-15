---
id: task-64
title: "Desk: GPT-Live front over the ChatGPT login (stretch)"
column: todo
position: 44
priority: low
tags:
  - voice
  - calls
  - providers
  - openai
  - stretch
  - 0.38.0
parentId: epic-6
dependsOn:
  - task-66
createdAt: "2026-10-14T20:03:59.244Z"
relatedFiles:
  - crates/hotline-core/src/voice/realtime/openai.rs
  - crates/hotline-core/src/voice/realtime/mod.rs
  - crates/hotline-core/src/voice/settings.rs
  - docs/voice-calls.md
  - docs/voice.md
updatedAt: "2026-10-15T03:58:03.454Z"
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/realtime/openai.rs
      description: "GPT-Live front: SDP exchange and sideband"
    - type: test
      path: crates/hotline-core/src/voice/realtime/openai.rs
      description: Protocol tests on local fixtures, feature flag, fail-soft
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy -p hotline-core --all-targets -- -D warnings
      - cargo test -p hotline-core
  constraints:
    - "Private route: behind a feature flag, fails soft to Grok or the pipeline"
    - Never echo provider bodies
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
---

## Description
Second provider. Desk POSTs the phone's SDP offer to chatgpt.com/backend-api/codex/realtime/calls (JSON {sdp, session}, model gpt-live-1-codex, delegation client, openai-alpha quicksilver=v2, bearer + chatgpt-account-id) and returns the answer; phone holds WebRTC media; desk attaches the sideband (wss://api.openai.com/v1/live/{call_id}, same identity) for delegation and context appends. Private route: feature-flag it, fail soft to Grok or the pipeline, and watch for an attestation requirement. API-key path (GA Realtime/GPT-Live) as the stable alternative.
