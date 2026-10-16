---
id: task-71
title: A teammate's own voice is picked from the voice front's voices
priority: medium
tags:
  - voice
  - calls
  - 0.38.0
relatedFiles:
  - crates/hotline-core/src/voice/duplex.rs
  - ui/src/components/Teammate.tsx
parentId: epic-6
createdAt: "2026-10-15T19:16:02.011Z"
completedAt: "2026-10-16T01:06:07.534Z"
updatedAt: "2026-10-16T01:06:07.534Z"
---

## Description
Done 2026-10-15, pending QA. voice.status reports front {providerId grok|openai|gpt-live, modelId, voice, voices}; the teammate card (desktop 1b971b1c, phone hotline-mobile 266a31b on feat/voice-front) lists the front's voices when calls run on one, else Legacy voice's; a front call speaks in the teammate's pick (voice_or_default keeps it to that front's voices). Harness: tests/voice.rs front call picks Rex; ui/tests/teammate-voice.test.tsx.
