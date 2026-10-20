---
id: task-225
title: "S6: work asked for on a call is done by the teammate's own model on the call's thread, without a word of hand-off"
column: backlog
position: 38
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-21
createdAt: "2026-10-19T20:00:07.934Z"
dependsOn:
  - task-224
updatedAt: "2026-10-20T20:13:22.833Z"
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
