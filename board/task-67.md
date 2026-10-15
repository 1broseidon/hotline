---
id: task-67
title: "Desktop: calls go through the voice front by default; the old pipeline becomes Legacy voice"
column: in-progress
position: 0
priority: high
tags:
  - voice
  - calls
  - ui
  - 0.38.0
parentId: epic-6
createdAt: "2026-10-15T04:47:02.607Z"
updatedAt: "2026-10-15T04:47:09.087Z"
---

## Description
George, 2026-10-14: build it now and test from the desk before the phone. Both desk calls and teammate calls use the front path when the desk advertises voiceFront; the transcribe-answer-speak pipeline stays as 'Legacy voice', chosen in Settings, and as the automatic fallback (no xAI login/key, front could not open, frontEnded, or an older desk). Phone keeps both paths and picks by what the desk advertises (hotline-mobile task-30).

Desk: voice.front absent now means Grok Voice on the first connected xAI login or key (was Off); {provider: legacy} is the pipeline. Window: Settings > Use for > Live calls picks Grok Voice (default) or Legacy voice; the Voice and Call assistant rows move under Legacy voice; Transcription stays (dictation uses it). ui/src/voice/call.ts: when voice.status has voiceFront, call_start asks mode front with inputRate 16000; with front in the reply the mic streams PCM16 continuously as voice.front_audio (increasing seq, fire and forget, dropped when refused), frontAudio plays gaplessly at outputRate, frontStop stops playback and answers voice.front_barge with playedMs, the Stop button barges too, frontEnded drops to the pipeline mid-call. Then make dev with George's Grok login and call both the desk and a teammate.
