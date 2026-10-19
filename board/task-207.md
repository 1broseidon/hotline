---
id: task-207
title: "Voice: a long call talks naturally, pointing at the chat only when it really holds more"
column: todo
position: 7
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/spoken.rs
  - crates/hotline-core/src/voice/dispatcher.rs
parentId: epic-17
dependsOn:
  - task-201
createdAt: "2026-10-19T14:02:18.044Z"
---

## Description
George, 2026-10-19 09:01 on a long call: "details are in the chat" comes out nearly every turn, which doesn't feel natural; a back-and-forth should just talk, and point at the chat only when it really holds more.

Where it comes from: every handed-off turn carries the same two-version instruction (voice/spoken.rs, the <spoken>/<written> block, 1-3 sentences, under 40 words, "details are in the chat when needed"), and the dispatcher's REWRITE prompt (voice/dispatcher.rs) tells the narrator the same. Nothing in either knows how far into the call it is, what was already said, or whether the written answer holds anything the spoken one can't carry.

Do:
- Pass the call's state into the per-turn instruction: how many turns in, whether the pointer was already said, and the last few spoken lines.
- Say the pointer only when the written version holds something speech can't carry (code, a table, links, a list longer than a few items) and at most once every few turns; never on conversational turns.
- Let a conversational turn's written version be the same words as the spoken one (no separate written answer to invent), so the chat doesn't fill with near-duplicates.
- Relax the 40-word cap when the person explicitly asks for an overview or says they have time.
Tests over the instruction builder and the narrator: no pointer on a conversational turn; at most one in N turns; present when the written version has code or a table.
