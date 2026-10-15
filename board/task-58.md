---
id: task-58
title: "Desk: realtime voice seam and Grok Voice adapter"
column: todo
position: 38
priority: high
tags:
  - voice
  - calls
  - providers
  - 0.38.0
parentId: epic-6
createdAt: "2026-10-14T20:03:56.458Z"
relatedFiles:
  - crates/hotline-core/src/voice/realtime/mod.rs
  - crates/hotline-core/src/voice/realtime/xai.rs
  - crates/hotline-core/src/voice/speech/xai.rs
  - crates/hotline-core/src/voice/mod.rs
updatedAt: "2026-10-15T01:57:04.230Z"
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/realtime/mod.rs
      description: Realtime trait and event types
    - type: file
      path: crates/hotline-core/src/voice/realtime/xai.rs
      description: Grok Voice adapter
    - type: test
      path: crates/hotline-core/src/voice/realtime
      description: Protocol tests on local fixtures
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy -p hotline-core --all-targets -- -D warnings
      - cargo test -p hotline-core
  constraints:
    - "Run cargo test -p hotline-core unfiltered (AGENTS.md: ts-rs contract)"
    - Reuse speech/xai.rs login/refresh/retry-once; never echo provider bodies
    - No Tauri in hotline-core; nothing unreachable ships
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
---

## Description
A Realtime trait beside Speech (voice/speech): open session, push PCM, receive audio/transcript/tool-call events, send tool results and text items, truncate on barge-in, close. First adapter: xAI wss://api.x.ai/v1/realtime, model grok-voice-latest, with the stored Grok login (xai-subscription, refresh + one retry on a rejected bearer, as speech/xai.rs does) or an API key. Handle session.update (instructions, voice, turn detection, audio formats, tools), 120-minute session limit (renew with a context carry-over), reconnect, errors without echoing provider bodies. Protocol tests on local fixtures. Confirm in the first live test: function calling, server VAD with interruption, input/output PCM rates.
