---
id: task-263
title: On a call, work_on_it refused "1 threads working" while the teammate looked idle
column: backlog
position: 33
priority: medium
tags:
  - bug
  - call
  - side-threads
relatedFiles:
  - crates/hotline-core/src/session/sides.rs
createdAt: "2026-10-21T23:12:05.446Z"
---

## Description
Seen 2026-10-21 ~22:30 on a phone call with Pork Chop (call 9e0c7eb9-a41c-4f00-8442-b5a071f5b9ab). George said "send the extension mockup to the chat". The call front ran work_on_it twice, a second apart, and both came back: "That teammate already has 1 threads working. Let one finish, then try again." list_schedules was empty and list_teammates doesn't list the caller itself, so the front had no way to see what held the place. ~15 s later the send went through on its own.

Likely: the side thread that had just produced extension mockup v2 was still mid-turn (posting at-the-wheel.jpg), so Sides::reserve found no idle side to park and refused. work_on_it refuses instead of queuing, unlike a handoff, which waits on has_room.

Odd: the message says 1, but MAX_LIVE is 3 in sides.rs on main and release/0.42.0. Either the desk that answered runs a different build, or something else caps the call's side threads at 1. Find out which first.

Done when:
- The cap in the refusal matches the cap actually enforced (test on the message text against MAX_LIVE).
- work_on_it from a call, when every place is mid-turn, queues the work (like a handoff) and says it will start when the current thread finishes, instead of refusing. Test: reserve with all sides working, then one finishing, starts the queued work.
- The front can see its own busy threads: the refusal or a lookup names what is running (title, how long), so it doesn't guess. Test on the refusal payload.
- Pluralisation: "1 thread", not "1 threads".
