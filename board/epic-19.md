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
---

## Description
George, 2026-10-19 12:17-12:19 (phone): split voice for maintainability, not build speed. Voice is a large body of code that has to be high quality, agents working on voice should stay in a voice crate rather than in one core, and the next iteration adds a voice agent, which should be built on a clean voice crate.

ADR 0007 planned only the engines (its step 3) and kept the calls in the core, because they report to the room through Weak<Room> (call_began, call_said, call_posted, call_ended, voice_push) and the dispatcher runs the wire's command table; docs/standards.md forbids a single-implementation trait to break the session/voice cycle. George's decision changes that for voice, with a test: the room's side of a call becomes a trait only if it comes out narrow (a handful of things a call needs and reports, implemented by the room and by test rooms, as RoomHandle is for the wire). If it would mirror most of the room, stop after the engines and keep the calls a module in the core; say so in the ADR.

Order: the engines move (mechanical); the call-interface design (stops in Review for George); the calls move only after he approves it; epic-17 (team lead and one voice path) runs after this epic, inside hotline-voice.

Same rules as epics 16 and 18: moves only, no behaviour change, contract.ts byte-identical, security.md citations and the module map follow, workspace lints, make check and make precheck before landing; a module that leaves renames its own crate:: paths.
