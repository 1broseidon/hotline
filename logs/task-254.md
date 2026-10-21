---
id: task-254
title: "Live call: a question asked over the voice's speech waits until the person speaks again"
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
    ended: "2026-10-21T08:39:45.265Z"
    outcome: done
assignee: codex
updatedAt: "2026-10-21T08:39:45.265Z"
decisions:
  - decision-16
completedAt: "2026-10-21T08:39:44.773Z"
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
- 2026-10-21T08:37:24.794Z: [codex] make check passed, including the full contract export and workspace tests; both-dialect socket regression and real-core voice wire test passed. Original adapter fails the new regression (sends response.cancel after provider speech_started). Added session/timing/state boundary traces, and recorded decision-16. make precheck is running. QA handoff: run ~/qa-audio/barge.sh on Grok Voice and GPT Realtime; correlate speech_started → speech_stopped → committed → response.created and verify the interrupting question is answered without another utterance. Scripted fixtures establish ordering, not real-provider VAD timing.
- 2026-10-21T08:39:33.792Z: [codex] Committed 25d9fc54 on dispatch/task-254; worktree clean. Guarded the caller's VAD commit/automatic-response sequence, removed duplicate VAD cancellation, added body-free provider boundary tracing, both-dialect scripted regressions and the wire capture-after-barge assertion. make check and all 8 make precheck legs passed. Ready for review; next action is Pork Chop's Linux ~/qa-audio/barge.sh pass on both Grok Voice and GPT Realtime before shipping (real-provider latency/root-cause confirmation remains pending).
- 2026-10-21T08:39:43.948Z: Landed on release/0.42.0
- 2026-10-21T13:51:42.228Z: [claude] 2026-10-21 Pork Chop: regression found by George's desk QA, fixed in dc89b550. The caller-turn guard assumed OpenAI's order (committed before response.created); Grok creates first, so the turn stayed open and every later desk reply (work results, notes, cards) was queued and never sent. A reply created before the commit now ends the turn at the commit, or at the reply's end. Test a_reply_created_before_the_commit_ends_the_caller_turn (both dialects) fails without the fix; rechecked on a real Grok call: 'Saved as ka sonnet.txt.' then it continued the sonnet.
