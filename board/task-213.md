---
id: task-213
title: "Design: the room's side of a call, and whether it is narrow enough to move the calls"
column: todo
position: 10
priority: high
tags:
  - design
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
  - docs/adr
parentId: epic-19
createdAt: "2026-10-19T17:20:19.224Z"
dependsOn:
  - task-212
updatedAt: "2026-10-19T17:20:20.185Z"
---

## Description
List exactly what the call code (call.rs, duplex.rs, front*.rs, dispatcher.rs, turn.rs, synthesis.rs, replies.rs, record.rs, input.rs, settings.rs) needs from the room and the rest of the core, and what it reports back: every Weak<Room> call, every crate:: reach, the dispatcher's use of wire::commands::run. Propose the smallest trait (or handed-in values plus a trait) that would let the calls live in hotline-voice: its methods, who implements it (the room, test rooms), and how the dispatcher reaches the command table without the wire (a narrow command trait or a handed-in executor). Apply George's test plainly: if the room's side is a handful of methods, recommend moving the calls; if it mirrors most of the room, recommend stopping. Also say what a future voice agent (a voice-native teammate) would build on, so the interface serves it. Write it into ADR 0007 as a new section. Docs only; stops in Review for George. --until review
