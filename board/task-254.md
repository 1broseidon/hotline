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
updatedAt: "2026-10-21T08:32:18.272Z"
decisions:
  - decision-16
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
- 2026-10-21T08:28:07.900Z: [codex] Found an adapter ordering hole: speech_started is forwarded but does not guard response.create; late tool/text completions can create a response before the caller's VAD commit. Provider docs say VAD owns cancellation/commit/automatic response. Adding both-dialect scripted ordering regression and body-free boundary tracing; live Linux QA remains necessary to confirm the reported provider stall.
- 2026-10-21T08:28:41.511Z: [claude] Narrowed (Pork Chop, a180a43d): an interruption over a spoken answer with NO tool calls before it was answered at once ('Maybe a fog horn would do instead' cut a musing reply and got 'True on cost…'). Every failing case had desk lookups (search_thread, list_chapters, read_thread) before the spoken answer. Look at what the tool-result path sends (function_call_output + response.create) and whether a late or duplicate response.create, or an un-cleared pending response, swallows the next caller turn after a truncate.
