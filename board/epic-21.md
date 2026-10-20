---
id: epic-21
title: "Voice (0.42): you talk to the teammate itself; the scenarios in docs/design/voice-scenarios.html are true"
type: epic
column: todo
position: 18
priority: high
tags:
  - voice
  - threads
  - "0.42"
assignee: claude
createdAt: "2026-10-19T20:00:05.973Z"
landsOn: release/0.42.0
updatedAt: "2026-10-20T20:13:20.963Z"
---

## Description
You talk to the teammate itself; voice only changes how. The acceptance set is docs/design/voice-scenarios.html, the scenario deck George walked through on 2026-10-20 at 15:09 ("everything is perfect"). Every story below lists its "Done when" lines, copied from the deck's "What must be true". A story is done only when every line is true in the code and proved by the test it names: a scripted-front test, a desk-harness test or a unit test. Where no test can reach (audio, the network, a phone), it's proved by a manual check recorded in the scenario pass (task-206).

The rule (ADR 0008 §5): voice changes how you talk to the teammate, never who does the work. The live voice talks and looks things up. Anything that writes or changes something is done by the teammate's own model and harness, as if typed.

George's three points from 2026-10-20 and the stories that fix them:
1. The chat doesn't know what "it" is: the front writes into the chat as if the chat had heard the call. Fixed by task-223 (call work on the call's own thread, replies on a call's post go to that thread, the home agent is handed the call's posts) and task-225 (posts stand alone).
2. "On it", "passing that along": the front is told it delegates, its tool is send_to_teammate, and its prompt carries those exact phrases (front_prompt.rs). Fixed by task-204 (the voice is the teammate) and task-225 (a first-person work tool).
3. The call's post is a blob: the five-minute summaries and the digest list turns. Fixed by task-223 (one closing note: a summary that opens the call).

Decisions (George accepted the deck with its leans, 2026-10-20):
- A reply on a call's post continues in the call's thread.
- Running work is steered by a line onto the clone's thread, never a second clone. "Never mind, drop it" stops it.
- Dictation is read back as a summary, with the full draft posted. Nothing is sent without the person saying so.
- Cards by voice: approvals and simple questions only. Never credentials, 2FA, screens, payments or deletions.
- A dropped call is held 20 s before it counts as a hang-up.
- A call that only chatted gets a one-line closing note.
- Legacy voice stays a choice and is the fallback.
- No separate decision model: the live model decides through its prompt and its tools, and code enforces the rules (the chat pointer, one post per turn, no repeated acknowledgement).

Order:
1. task-214 S0: the calls move into hotline-voice first. George, 2026-10-20: "extract voice into its own crate". This reverses ADR 0008 §5's "after slice 6", so every slice after it works inside hotline-voice.
2. task-223 S5.
3. task-224, the proving call.
4. task-225 S6.
5. In parallel once S6 lands: task-204, task-207, task-239 (cards), task-203 (desk and lead).
6. task-240 and task-241 (trouble), once S5 lands.
7. task-206, the end to end, docs and scenario pass, last.

Phone stories are on the hotline-mobile board, in the epic that pairs with this one.

Lands on release/0.42.0. The 0.42 cut waits for this epic (George, 2026-10-20: "before we cut anything"). Pork Chop QAs on the Linux QA computer first, then George.

## Log
- 2026-10-20T06:51:49.934Z: [claude] Runs after epic-22 (Rig 0.45 upgrade) reaches S3 and S6: S1 rewrites the voice dispatcher's hooks and S6 its metering, which slice 6 of this epic reshapes.
