---
id: decision-16
title: Provider VAD owns the caller's commit and automatic answer.
type: decision
status: current
claimKey: voice.vad-turn-ownership
kind: decision
holdsWhile: Grok Voice and GPT Realtime use provider VAD with automatic responses.
recheck: If these adapters switch to manual turn detection or disable automatic response creation.
tags:
  - voice
  - realtime
createdAt: "2026-10-21T08:32:18.271Z"
---

## Description
Provider VAD owns the caller's commit and automatic answer.
After speech_started, late tool outputs and text may add conversation context but may not send response.create before the committed turn's automatic response.created. That answer consumes the pending context once. Provider-detected barge-in truncates playback without duplicating VAD cancellation; taps still cancel active generation. Never clear or manually commit microphone input as a playback-interruption workaround. Preserve decision-9's distinction between generated and heard speech. Boundary traces carry timing and state, never audio, transcript or tool contents.
