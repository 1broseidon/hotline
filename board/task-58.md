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
---

## Description
A Realtime trait beside Speech (voice/speech): open session, push PCM, receive audio/transcript/tool-call events, send tool results and text items, truncate on barge-in, close. First adapter: xAI wss://api.x.ai/v1/realtime, model grok-voice-latest, with the stored Grok login (xai-subscription, refresh + one retry on a rejected bearer, as speech/xai.rs does) or an API key. Handle session.update (instructions, voice, turn detection, audio formats, tools), 120-minute session limit (renew with a context carry-over), reconnect, errors without echoing provider bodies. Protocol tests on local fixtures. Confirm in the first live test: function calling, server VAD with interruption, input/output PCM rates.
