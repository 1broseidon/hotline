---
id: task-207
title: The chat gets results, records and one closing note; the pointer only when the chat holds more
column: backlog
position: 42
priority: high
tags:
  - voice
assignee: codex
relatedFiles:
  - crates/hotline-core/src/voice/spoken.rs
  - crates/hotline-core/src/voice/dispatcher.rs
parentId: epic-21
dependsOn:
  - task-225
createdAt: "2026-10-19T14:02:18.044Z"
updatedAt: "2026-10-21T06:53:09.067Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T06:53:08.028Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T06:53:09.067Z"
---

## Description
Scenarios: Where are we on it?, Thinking out loud, and every "In the chat afterwards". George, 2026-10-19: "details are in the chat" came out nearly every turn.

Done when:
- Code, not the model, decides whether a reply points at the chat, from what the post holds: a file, code, a table, links, or a list longer than a few items. The pointer is said at most once every few turns, and never on a conversational turn.
- A conversational turn's written version is its spoken words. There is no second answer to invent, and no near-duplicates in the chat.
- The per-turn instruction (Legacy voice and the narrator) carries the call's state: how many turns in, whether the pointer was said, and the last few spoken lines. The fixed 40-word cap and the "details are in the chat when needed" line in voice/spoken.rs and the dispatcher's REWRITE prompt are gone.
- On a call, any reply whose written version carries what speech can't is posted by the core, decided by what the text contains (ADR 0008 §4).

Proved by tests over the instruction builder and the narrator: no pointer on a conversational turn, at most one in N turns, and a pointer present when the written version has code or a table.

## Log
- 2026-10-19T14:02:58.358Z: [claude] George, 09:02, refines the rule: the pointer is for something real the person will want later, like a file sent ('I sent you the HTML file, take a look when you're back at your desk'), a link, or a long write-up. It's said in context, in the teammate's own words, never a fixed phrase, the way a smart coworker on the other end of the line would mention it. A plain answer or a short list is just said, with no pointer. Put this into the instruction and the narrator prompt as a description plus bad examples, not stock good phrases (same approach as task-204).
