---
id: task-254
title: "Live call: a question asked over the voice's speech waits until the person speaks again"
column: in-progress
position: 0
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T08:25:55.990Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T08:26:09.773Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T08:26:11.298Z"
assignee: codex
updatedAt: "2026-10-21T08:26:31.746Z"
---

## Description
Found by Pork Chop on the Linux QA desk, 2026-10-21, real Grok Voice calls through the virtual mic. Present at 1f3a2cae as well as the tip 2735d01a, so it predates tonight's landings.

Repro (~/qa-audio/barge.sh on the QA computer): on a live call say 'I have time. Give me the overview of the files you wrote tonight, in numbered parts.' While the spoken rundown plays, about 26 s in, say 'Wait. Which one was the newest?'

Seen: speech stops at once and the thread records 'Speech interrupted. Playback stopped at …' (correct). Then no user line and no answer, for over 90 s. Once the front called remain_silent. The interrupting words were not lost: the next thing said was merged with them into one transcript ('Hold on. Just tell me how many calls that was. How many calls was that?') and answered. Once (790a5312) the answer came after about 30 s.

Not the cause: the window sends mic frames continuously (ui/src/voice/call.ts hearFront), the desk forwards them unconditionally (duplex front_audio), and the virtual mic sends silence between utterances. Plain turns and an interruption with 'Stop. Just tell me the keeper's name.' over a spoken story were answered at once earlier tonight.

Suspect: the provider doesn't commit the caller's turn after our conversation.item.truncate (front_barge / SpeechStarted → cut_speech), or a response.create we send after the cut collides with the open caller turn. Needs provider-event tracing (Grok Voice and GPT Realtime) to pin down.

Done when: a question asked over the voice's speech is heard and answered within a normal turn's time, on Grok Voice and GPT Realtime, with a scripted-front test that reproduces the commit sequence.

## Log
- 2026-10-21T08:26:08.801Z: [claude] For the Codex run: read crates/hotline-voice/src/realtime (socket, live) and duplex.rs for what we send after a barge (truncate, response.create, function outputs, any input_audio_buffer.clear/commit) and compare with each provider's documented barge-in sequence. Fix the sequence, add a scripted-front test that reproduces it. Pork Chop will verify on the Linux QA desk with ~/qa-audio/barge.sh before it ships.
