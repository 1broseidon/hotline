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

## Log
- 2026-10-19T17:20:45.418Z: [claude] George, 12:20, sets the direction this design must follow: voice is a modality, not a separate agent. When you call a teammate, the teammate itself is the one talking: the full agent, with its tools and everything it does, with speech in and speech out instead of text. A summary goes back to the chat when needed, as today. That reverses ADR 0005's design, where a separate live-voice front model answers and looks things up itself and hands only heavy work to the teammate. So the design should: (1) define the voice layer as I/O and turn-taking only (speech to text or a speech-native model in; streamed speech out; voice activity detection, barge-in, a quick acknowledgement or sound while the agent works, the end-of-call summary), with the agent unchanged behind it; (2) say what happens to the live fronts (Grok Voice, GPT Realtime, GPT-Live): kept as speech transports only, or as the brain of a Hotline Agent teammate that runs on a realtime model (the coming voice agent), never as a second agent in front of the teammate; (3) handle latency for slow agents (an ACP teammate's turn can take many seconds): stream the reply as it's written, acknowledge at once, let the person interrupt; (4) re-check the room interface against this: a modality layer may need far less of the room than today's fronts do. Recommend whether the call code then fits in hotline-voice.
