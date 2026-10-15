---
id: task-64
title: "Desk: GPT-Live front over the ChatGPT login"
column: todo
position: 44
priority: low
tags:
  - voice
  - calls
  - providers
  - openai
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
updatedAt: "2026-10-15T18:49:54.495Z"
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
Built 2026-10-15 on release/0.38.0 (618a827a), pending George's QA. voice/realtime/live.rs places the call as the Codex app does: POST chatgpt.com/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas with the ChatGPT login's bearer + ChatGPT-Account-ID, openai-alpha quicksilver=v2, JSON {sdp, session{model gpt-live-1-codex, instructions, audio.output.voice, delegation{type client}}}; call id from Location; sideband wss://api.openai.com/v1/live/<call id>. The desk (not the phone) is the WebRTC peer (webrtc 0.20.3 + opus 0.4, as Codex pins): mic PCM 24k doubled to 48k, Opus 20 ms packets paced with silence fill; host candidates only. The sideband carries speech as 24 kHz PCM (session.output_audio.delta), transcripts, turns and delegation.created, so no Opus decode. delegation.created -> the bridge's send_to_teammate; a line to say -> session.context.append speakable (said in the model's own words, not verbatim); person's turn -> barge-in. Lines wait for session.started; create retried on 5xx/unreachable; sideband rejoined; ended call or failed peer drops the front to the pipeline. Free against the Voice budget. Voices: cove (default), juniper, maple, spruce, ember, vale, breeze, sol. Settings: Live calls > Model > OpenAI > GPT-Live (Included with your ChatGPT subscription). Proof: live/tests.rs unit tests + ignored live test (HOTLINE_TEST_CHATGPT_DIR, HOTLINE_TEST_ASK_WAV) passed 3/3 against the real service; make check green. Build needs cmake (docs/development.md). Not done: 60-minute session renewal (falls back to pipeline), attestation header (not required today).
