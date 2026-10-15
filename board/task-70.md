---
id: task-70
title: "Desk: GPT Realtime front on an OpenAI API key"
column: review
position: 7
priority: high
tags:
  - voice
  - calls
  - providers
  - openai
  - 0.38.0
relatedFiles:
  - crates/hotline-core/src/voice/realtime/socket.rs
  - crates/hotline-core/src/voice/speech/providers.rs
parentId: epic-6
createdAt: "2026-10-15T18:50:11.566Z"
---

## Description
Done 2026-10-15 (1040bc01, 3109fc16, f89377eb), QA'd by George. voice/realtime/socket.rs is one driver for both OpenAI-shaped realtime sockets (xai.rs renamed): Grok and GPT Realtime (gpt-realtime-2.1, 24 kHz only, 60-min sessions renewed at 55, gpt-4o-mini-transcribe for the person's words). Charged $0.10/min against the Voice budget. Bug found in QA and fixed: GPT took 'Say exactly this…' lines as the person's words and relayed them back, looping the teammate; now lines go in as system items with tool_choice none, and the bridge refuses any relay that echoes a line it was given (both dialects). OpenRouter has no realtime speech API, so it is not offered.
