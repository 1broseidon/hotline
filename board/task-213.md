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
  - task-215
updatedAt: "2026-10-19T18:23:50.235Z"
contract:
  status: ready
  deliverables:
    - type: design
      path: docs/adr/0007-split-hotline-core.md
      description: "a section headed \"The room's side of a call\": the inventory, the proposed trait or values, the command-table answer, the voice agent's footing, and a plain recommendation (move or stop)"
  validation:
    commands:
      - grep -q "^## The room's side of a call" docs/adr/0007-split-hotline-core.md
      - grep -qiE '^\*\*Recommendation' docs/adr/0007-split-hotline-core.md
      - git diff --quiet release/0.41.0...HEAD -- crates ui Cargo.toml Cargo.lock
      - python3 scripts/security-citations
  constraints:
    - "Docs only: no change under crates/, ui/, Cargo.toml or Cargo.lock"
    - "The inventory cites file:line on release/0.41.0 after task-212 for every Weak<Room> call and crate:: reach it counts"
    - "The recommendation applies George's test in one sentence: move if the room's side is a handful of methods, stop if it mirrors most of the room"
  metrics:
    readyAt: "2026-10-19T17:22:11.096Z"
---

## Description
List exactly what the call code (call.rs, duplex.rs, front*.rs, dispatcher.rs, turn.rs, synthesis.rs, replies.rs, record.rs, input.rs, and voice/mod.rs's own items) needs from the room and the rest of the core, and what it reports back: every Weak<Room> call, every crate:: reach, the dispatcher's use of wire::commands::run. Propose the smallest trait (or handed-in values plus a trait) that would let the calls live in hotline-voice: its methods, who implements it (the room, test rooms), and how the dispatcher reaches the command table without the wire (a narrow command trait or a handed-in executor). Apply George's test plainly: if the room's side is a handful of methods, recommend moving the calls; if it mirrors most of the room, recommend stopping. Also say what a future voice agent (a voice-native teammate, epic-17) would build on, so the interface serves it. Write it into ADR 0007 as a new section. Docs only; stops in Review for George. --until review

Lead's additions (2026-10-19):
- Work on release/0.41.0 after task-212 landed: the engines are in crates/hotline-voice, and the ADR lists their old and new paths. Read task-212's notes for the re-exports the core still keeps.
- The section is headed exactly `## The room's side of a call` and contains a line starting `**Recommendation:**` that says move or stop in one sentence. Cite file:line for each reach you count, and give the count (how many methods, how many of the room's public methods that is).
- Sketch the trait as Rust signatures in the ADR, and say for each method which call file uses it. If you recommend moving, also say how big the move is and whether it should be one change or two (for example: the trait and the room's impl first, then the files), so the lead can split task-214.
- Write no code. The lead does not hand off the calls' move until George approves this section.

Carried from task-212 (landed 2026-10-19):
- The vault reaches the engines as a value, `speech::Connected { root, providers, sign_ins }`, which the core's voice/connected.rs reads. The calls (input.rs, duplex.rs) still call `speech::resolve*(&self.vault, …)` through the core's five &Vault readers (resolve, resolve_output, resolve_front, listens_for, options). Count those reaches. In the design, the calls take a Connected, or the room hands one in, and voice/connected.rs is what's left in the core. Say which.
- Re-exports the core still keeps while callers remain: crate::voice::speech (a glob of hotline_voice::speech plus the five readers), crate::voice::realtime, crate::voice::front::SEND_TO_TEAMMATE (now hotline_voice::realtime's), crate::models::is_custom (now hotline_room::provider_auth's). Say which of them go away when the calls move.
- xAI's bearer and ChatGPT's refresh reach the engines as fn pointers in `speech::SignIns`, pointed at providers in the core (one process-wide xAI token lock shared with Rig). The interface should leave that in place.
- task-215 (epic-20's agent-thread ADR, in Review for George) has a section on voice under the thread layer. Read it once it has landed, and keep this interface consistent with it.

## Log
- 2026-10-19T17:20:45.418Z: [claude] George, 12:20, sets the direction this design must follow: voice is a modality, not a separate agent. When you call a teammate, the teammate itself is the one talking: the full agent, with its tools and everything it does, with speech in and speech out instead of text. A summary goes back to the chat when needed, as today. That reverses ADR 0005's design, where a separate live-voice front model answers and looks things up itself and hands only heavy work to the teammate. So the design should: (1) define the voice layer as I/O and turn-taking only (speech to text or a speech-native model in; streamed speech out; voice activity detection, barge-in, a quick acknowledgement or sound while the agent works, the end-of-call summary), with the agent unchanged behind it; (2) say what happens to the live fronts (Grok Voice, GPT Realtime, GPT-Live): kept as speech transports only, or as the brain of a Hotline Agent teammate that runs on a realtime model (the coming voice agent), never as a second agent in front of the teammate; (3) handle latency for slow agents (an ACP teammate's turn can take many seconds): stream the reply as it's written, acknowledge at once, let the person interrupt; (4) re-check the room interface against this: a modality layer may need far less of the room than today's fronts do. Recommend whether the call code then fits in hotline-voice.
