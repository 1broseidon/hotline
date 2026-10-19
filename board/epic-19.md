---
id: epic-19
title: "hotline-voice: the voice system as its own crate, engines first, then the calls behind a narrow room interface (0.41)"
type: epic
column: todo
position: 8
priority: high
tags:
  - crate-split
  - voice
  - "0.41"
assignee: claude
createdAt: "2026-10-19T17:20:17.780Z"
landsOn: release/0.41.0
updatedAt: "2026-10-19T20:00:16.666Z"
---

## Description
George, 2026-10-19 12:17-12:19 (phone): split voice for maintainability, not build speed. Voice is a large body of code that has to be high quality, agents working on voice should stay in a voice crate rather than in one core, and the next iteration adds a voice agent, which should be built on a clean voice crate.

ADR 0007 planned only the engines (its step 3) and kept the calls in the core, because they report to the room through Weak<Room> (call_began, call_said, call_posted, call_ended, voice_push) and the dispatcher runs the wire's command table; docs/standards.md forbids a single-implementation trait to break the session/voice cycle. George's decision changes that for voice, with a test: the room's side of a call becomes a trait only if it comes out narrow (a handful of things a call needs and reports, implemented by the room and by test rooms, as RoomHandle is for the wire). If it would mirror most of the room, stop after the engines and keep the calls a module in the core; say so in the ADR.

Order: the engines move (mechanical); the call-interface design (stops in Review for George); the calls move only after he approves it; epic-17 (team lead and one voice path) runs after this epic, inside hotline-voice.

Same rules as epics 16 and 18: moves only, no behaviour change, contract.ts byte-identical, security.md citations and the module map follow, workspace lints, make check and make precheck before landing; a module that leaves renames its own crate:: paths.

## Log
- 2026-10-19T17:21:00.198Z: [claude] Lead: hand off task-212 (engines) and land it. task-213 (design) is handed off --until review and is not landed: it waits in Review for George, and task-214 does not start until he approves it. If the design recommends stopping (the room's side isn't narrow), task-214 is not started either.
- 2026-10-19T17:22:36.695Z: [claude] Lead's plan: strictly serial, on release/0.41.0. (1) task-212 moves the engines (speech, realtime) to crates/hotline-voice; it goes first because the design has to inventory the calls' reaches against the engines' new home, and both edit ADR 0007. (2) task-213 writes the call-interface design into ADR 0007 under '## The room's side of a call' with a '**Recommendation:**' line; docs only, validated to touch nothing under crates/ui. (3) task-214 moves the calls and is the epic's whole-system proof (engines plus calls in hotline-voice, room implementing the trait, full voice/local_speech/processes harnesses, make check, make precheck). GATE: in bypass task-213 lands on its own, so the lead holds task-214 until George approves the design in a note or message; if the design says stop, task-214 is not handed off and George decides it. The lead may split task-214 into the trait-plus-impl and the file move if task-213 sizes it as two. All three now carry validation commands (file absences, no hotline-core/Tauri dependency, cargo test -p hotline-voice, the harnesses, make check, contract.ts unchanged, make precheck). Agents: claude/opus, high effort for the move, xhigh for the design.
- 2026-10-19T18:23:50.541Z: [claude] task-212 landed on release/0.41.0: engines in crates/hotline-voice, no new trait, vault handed in as speech::Connected. Its carry-overs (the &Vault readers the calls still use, the re-exports the core keeps, the SignIns fn pointers) are now in task-213's description. task-213 waits on task-215 (epic-20, in Review for George), so nothing is handed off until George lands that.
- 2026-10-19T20:00:15.224Z: [claude] epic-19 is done for 0.41 with the engines' move (task-212). task-213 is closed into task-214, which moved to epic-21 (0.42) after slice 6.
