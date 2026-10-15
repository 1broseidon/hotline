---
id: task-66
title: "Desk: wire the voice front end to end, then test it live on Grok"
column: in-progress
position: 0
priority: critical
tags:
  - voice
  - calls
  - integration
  - 0.38.0
parentId: epic-6
createdAt: "2026-10-15T03:57:56.431Z"
assignee: claude
updatedAt: "2026-10-15T04:03:57.141Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: person
  at: "2026-10-15T04:03:40.391Z"
---

## Description
The missing piece of epic-6. Tasks 57-63 each landed a tested part on release/0.38.0 but every one deferred its wiring to 'the next task'. Done when a front call from the desk window (or a headless harness in crates/hotline-core/tests) talks to Grok Voice with the teammate doing the work.

Wire (from the task notes):
1. Call start reads VoiceSettings.front (task-63, voice/settings.rs FrontChoice); resolve GrokVoice from the vault (GrokVoice::with_subscription when the credential is xai-subscription, else the API key) into Services.realtime, since Calls::realtime() reads only that (task-61 note 3). No resolver exists in speech/providers.rs yet.
2. voice/duplex.rs run_front: replace the placeholder INSTRUCTIONS with front_prompt.rs (task-60; caller passes only spoken DM lines, no tool output) and pass the bridge's tools (front.rs tools(): send_to_teammate, remain_silent, mapped to RealtimeTool); route RealtimeEvent::ToolCall to Front::tool_call and send its output as tool_result instead of 'not available'.
3. Feed Front::next_note() (Spoken / Progress / NeedsYou) back to the session; Spoken must be said verbatim (xAI force_message item, noted in task-58, or a 'say exactly this' text item).
4. Set Call.answering so thinking + listening:true shows the teammate working (task-61 note 2).
5. Ledger/record hooks (task-62): front_ready before opening, front_spend about once a minute, front_heard/front_said from RealtimeEvent::Heard/Saying.
6. Then a LIVE test against Grok (task-58 left unconfirmed): function calling, input transcription arriving without audio.input.transcription, server VAD barge-in, 16 kHz in / 24 kHz out. If any fails, stop and replan.
Contract: fmt, clippy -D warnings, UNFILTERED cargo test -p hotline-core, ui typecheck/test, a headless harness test for the wired path. Branch off release/0.38.0; merge back into it (never local main). Phone half is hotline-mobile task-30.
