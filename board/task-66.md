---
id: task-66
title: "Desk: wire the voice front end to end, then test it live on Grok"
priority: critical
tags:
  - voice
  - calls
  - integration
  - 0.38.0
parentId: epic-6
createdAt: "2026-10-15T03:57:56.431Z"
assignee: claude
updatedAt: "2026-10-15T17:07:49.030Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: person
  at: "2026-10-15T04:03:40.391Z"
column: in-progress
position: 0
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

## Log
- 2026-10-15T04:03:59.610Z: [claude] Dispatch branch started from main; fast-forwarded dispatch/task-66 to release/0.38.0 (main's tip is its ancestor) so the work sits on the release as the task asks.
- 2026-10-15T04:20:10.241Z: [claude] Wired on dispatch/task-66 (019c0ffb, on top of release/0.38.0): speech::resolve_front picks Grok Voice from the vault (named login/key, else first connected) and Calls::realtime() uses it; front_ready gates the front at call start; run_front opens with front_prompt (name, last chapter note, DM lines) and the bridge's two tools, routes ToolCall to front::Front::tool_call (empty remain_silent output asks for no reply), says Front::next_note() via a 'say exactly this' text item, records front_heard/front_said, charges front_spend each minute and on stop (spent budget ends the call, revoked login drops the front). Relays now pass on front calls only; the desk synthesizes no clips while a front speaks; answering/listening comes from hand_off as before; progress lines at most every 45 s; the front's own said ids are front-* and the bridge ignores them. Harness: tests/voice.rs a_front_call_relays_to_the_teammate_and_says_its_spoken_line; make check green. LIVE TEST NOT RUN: no xAI API key here and the only xAI credential is George's OAuth login in the keychain, which I did not touch. Run: HOTLINE_LIVE_XAI_KEY=... cargo test -p hotline-core --test voice live -- --ignored --nocapture (checks transcription without audio.input.transcription, function calling, 16k in/24k out frames, VAD barge-in). force_message is undocumented in xAI's public docs, so verbatim speech relies on the text item. Residual: the card shows the teammate's spoken line and the front's repeat of it as two said lines.
- 2026-10-15T04:38:19.170Z: [claude] 2026-10-14 live test run by Pork Chop with George's Grok subscription login (OAuth bearer as HOTLINE_LIVE_XAI_KEY): PASS. Grok transcribed 16 kHz speech without audio.input.transcription ('Ask Mac to check the failing PR.'), called send_to_teammate, the teammate got the words unrephrased, the front said the teammate's spoken line verbatim ('The checks passed.') in 100 ms 24 kHz frames, and talking over it gave frontStop. Two test fixes: retry loop now sends increasing seq (the desk consumes a seq even when it answers 'still connecting'; the phone must do the same), and the name check accepts 'Mac'. Merged (fast-forward) into release/0.38.0 and pushed; clippy + unfiltered core tests green (1885). Residuals: the call card shows the teammate's spoken line and the front's repeat as two said lines; the front adds filler ('I've passed that along').
- 2026-10-15T17:07:49.030Z: Reopened into In progress
