---
id: task-58
title: "Desk: realtime voice seam and Grok Voice adapter"
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
updatedAt: "2026-10-15T02:10:10.808Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-15T01:57:25.510Z"
    reworkCount: 0
    deliveredAt: "2026-10-15T02:10:09.706Z"
    duration: 764
assignee: claude
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-15T01:57:15.439Z"
completedAt: "2026-10-15T02:10:10.808Z"
---

## Description
A Realtime trait beside Speech (voice/speech): open session, push PCM, receive audio/transcript/tool-call events, send tool results and text items, truncate on barge-in, close. First adapter: xAI wss://api.x.ai/v1/realtime, model grok-voice-latest, with the stored Grok login (xai-subscription, refresh + one retry on a rejected bearer, as speech/xai.rs does) or an API key. Handle session.update (instructions, voice, turn detection, audio formats, tools), 120-minute session limit (renew with a context carry-over), reconnect, errors without echoing provider bodies. Protocol tests on local fixtures. Confirm in the first live test: function calling, server VAD with interruption, input/output PCM rates.

## Log
- 2026-10-15T02:10:09.398Z: [claude] Done on branch dispatch/task-58 (b507e1fd). voice::realtime: `Realtime` trait (id, is_subscription, open(RealtimeConfig) -> RealtimeSession), session = cloneable `RealtimeInput` (push_audio PCM16, tool_result, text, truncate(played_ms)) + `events: Receiver<Result<RealtimeEvent, SpeechError>>` (Audio, SpeechStarted, Heard, Saying, ToolCall, ResponseDone, Refused{code only}, Renewed). Dropping all inputs closes; an Err ends the stream. Adapter `GrokVoice::new(endpoint)[.with_subscription(tokens)]`, model grok-voice-latest; login/refresh-once/no-body errors now in shared `speech::xai::Access`. Driver owns the socket: server_vad always; a tool result/text sent mid-reply waits and response.create goes after response.done; renews at 115 min (119 if mid-reply) and reconnects on a lost socket (3 retries, Unreachable only), carrying a 16 KB "Caller:/You:" transcript in the instructions; emits Renewed. For next tasks: nothing constructs GrokVoice in production yet (task-59/61 wire it; no resolver in speech/providers.rs). xAI has a `force_message` item for verbatim speech (task-59/60's "say the teammate's line verbatim") not yet exposed. Unconfirmed until a live test: user transcript arrives as conversation.item.input_audio_transcription.completed without configuring audio.input.transcription; function calling; barge-in quality; 16k in / 24k out rates. Carry keeps a truncated reply's full transcript (residual).
