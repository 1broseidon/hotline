---
id: task-255
title: "Live dictation: a pause inside 'draft… / say… / write…' ends the turn and starts work on half a sentence"
column: in-progress
position: 1
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T08:37:50.053Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T08:49:43.268Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T08:49:45.959Z"
assignee: codex
updatedAt: "2026-10-21T08:52:51.247Z"
decisions:
  - decision-18
---

## Description
Linux QA, 2026-10-21, Grok Voice, 2735d01a: 'Draft a message to the harbour master saying that' + 3.5 s pause + 'the fog horn should be tested on Friday morning, before the boats go out.' The provider's VAD ended the turn at the pause; the front called work_on_it with the half sentence and guessed content ('saying that: the harbor note…'), then the second half steered it. The prompt's remain_silent rule for unfinished phrases isn't enough: the turn boundary is the provider's. Done when: a pause of up to ~4 s inside an unfinished dictation phrase doesn't start work (e.g. a longer server-VAD silence window while a dictation verb is open, or holding work_on_it until the phrase completes), on Grok Voice and GPT Realtime, with a scripted-front test.

## Log
- 2026-10-21T08:52:24.073Z: [codex] Implementing a shared live-front work admission guard based on caller transcript, with silent rejection of dangling dictation and deferred admission when tool calls beat transcription. Provider VAD ownership stays unchanged; scripted Grok/OpenAI regression covers a four-second pause and invented completion.
- 2026-10-21T08:56:17.555Z: [codex] Recorded decision-18 (voice.unfinished-dictation), preserving decision-16. Guard and late-transcription ordering are implemented with lexical unit coverage and a real-core scripted-front test for Grok/OpenAI, including four seconds of silence, guessed arguments, continuation, and withdrawal of queued work. Full check/precheck underway; live microphone QA remains the physical-provider verification.
