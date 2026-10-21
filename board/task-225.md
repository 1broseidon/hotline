---
id: task-225
title: "S6: work asked for on a call is done by the teammate's own model on the call's thread, without a word of hand-off"
column: review
position: 0
priority: high
tags:
  - voice
assignee: codex
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-21
createdAt: "2026-10-19T20:00:07.934Z"
dependsOn:
  - task-224
  - task-244
updatedAt: "2026-10-21T05:38:06.399Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T05:05:35.580Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T05:05:37.529Z"
---

## Description
Scenarios: Asking for real work, Changing your mind mid-work, Dictating something, and Thinking out loud where it ends in "write that down". ADR 0008 §5 and §6 slice 6.

Done when:
- The live voice holds one work tool. It is named and described in the first person, as the teammate's own action (write, change, run), never as sending, passing, relaying or a teammate.
- Calling the work tool starts a clone of the teammate (ADR 0008 §0) on the teammate's own model and harness, focused on the task, on the call's thread. The tool returns at once.
- send_to_teammate into the DM, the relay (hand_off, CALL_ORIGIN) and the call assistant's delegation loop (dispatcher::answer_delegation) are gone. GPT-Live's delegation is answered by the same work tool.
- The person can keep talking while the work runs, and lookups still answer.
- The home agent is never interrupted by a call's work.
- The clone's result comes back into the call to be said. What it made is posted to the chat as a post that stands alone: it names its subject and carries the file. The pointer to the chat is said once, because there is something in it.
- A change to running work steers it with a line onto the clone's thread, and never starts a second clone. The confirmation repeats back what changed in a few words.
- "Never mind, drop it" stops the work and says what was left unfinished.
- During dictation, a pause inside one utterance is not the end of the turn: "draft…", "say…" and "write…" open a longer listening window. The draft is read back as a summary, and the full text is posted. Nothing is sent on the person's behalf without them saying so.
- When a conversation ends in "write that down", the clone gets the whole call so far as its brief.

Proved by scripted-front tests on Grok Voice and GPT Realtime, plus GPT-Live's delegation, and desk-harness tests for the clone and the posts. The listening window is checked by hand in the scenario pass (task-206). ADR 0005 is marked amended.

Added 2026-10-20 (George, after the S5 proving call; see task-244):
- Everything said on a live-voice call, by both sides, is recorded on the call's thread in order as its transcript, with each lookup and each piece of work as its own tool line. The thread reader (task-244) returns it like any call.

## Log
- 2026-10-21T05:11:18.917Z: [codex] Replacing the live-front DM relay with the existing call-thread occupant, including GPT-Live. Lookups now bind to the call thread; work uses the teammate's selected model and existing driver, with the whole call as its brief. Removing direct front writes and raw MCP exposure so changes go through the single work tool. Legacy desk routing remains task-203's separate dispatcher path.
- 2026-10-21T05:28:45.600Z: [codex-task-225] The own-thread clone/file-post matrix passes on Hotline Agent and ACP with scripted Grok, GPT Realtime and GPT-Live; correction/cancellation leaves home work running. Background command wakes now stay on their originating thread and desk process controls can reach them. Both speakers and lookup/work tool admissions are recorded in order, including interrupted speech. Full check/precheck are running; task-206 must still exercise real dictation pauses (OpenAI semantic VAD low eagerness, Grok/GPT-Live listening instructions) and spoken confirmations.
- 2026-10-21T05:38:06.115Z: [codex-task-225] Ready for review on dispatch/task-225: a2226b60, 990ddecd, f0d1aa7f, bab0e71e; worktree clean. One first-person work_on_it starts/steers/stops the own-model/harness call-thread clone; live speech and lookups continue, home work is untouched, the full call briefs work, and files/full drafts post with subjects. Removed the direct-call DM relay, CALL_ORIGIN, hand_off and answer_delegation; GPT-Live invokes the same tool, including stop. Ordered speech/tool transcripts include interrupted speech and preserve desktop provenance. ADR 0005 marked amended. make check and all 8 make precheck legs passed. Hand checks explicitly handed to task-206: real dictation pauses, acknowledgment/correction phrasing and one chat pointer; Grok/GPT-Live use native turn detection plus listening instructions, OpenAI semantic VAD low eagerness. Legacy desk dispatch remains task-203; nothing else left unwired.
