---
id: epic-21
title: "Voice (0.42): you talk to the teammate itself; the scenarios in docs/design/voice-scenarios.html are true"
type: epic
priority: high
tags:
  - voice
  - threads
  - "0.42"
assignee: claude
createdAt: "2026-10-19T20:00:05.973Z"
landsOn: release/0.42.0
updatedAt: "2026-10-21T21:35:02.064Z"
completedAt: "2026-10-21T21:35:02.064Z"
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

Phone stories are hotline-mobile epic-5 (task-43 to task-46), in step with these.

Lands on release/0.42.0. The 0.42 cut waits for this epic (George, 2026-10-20: "before we cut anything"). Pork Chop QAs on the Linux QA computer first, then George.

## Log
- 2026-10-20T06:51:49.934Z: [claude] Runs after epic-22 (Rig 0.45 upgrade) reaches S3 and S6: S1 rewrites the voice dispatcher's hooks and S6 its metering, which slice 6 of this epic reshapes.
- 2026-10-20T20:13:59.071Z: [claude] Rewritten 2026-10-20 from the scenario deck George accepted: 11 children, each with Done-when lines that must be true in code and proved by named tests. S0 (task-214, extract the calls into hotline-voice) now goes first, at George's call. Phone: hotline-mobile epic-5.
- 2026-10-21T09:09:53.913Z: [claude] 2026-10-21 Pork Chop: all 19 children done and on release/0.42.0 (tip 0e5b28bb). The Linux scenario pass is recorded in docs/design/voice-scenario-pass.md; desk and phone passes are George's. Moved to Review: awaiting George's QA before the 0.42 cut. Open decision: task-253 (files written on a call reaching the chat).
- 2026-10-21T14:00:57.533Z: [claude] 2026-10-21 09:00 George desk QA: checklist 2 (talk over a long answer) passed; 1 (real work on a call) failed — a Grok ordering regression from task-254 — fixed in dc89b550 and passed on George's retest, including continuing the sonnet after saving.
- 2026-10-21T14:28:32.331Z: [claude] 2026-10-21 09:28 George desk QA: item 6 (calling the desk) passed — Goldie as lead handed a named task to Huck (side thread research) and answered 'what did we do this week' while it ran. Items 1, 2, 4, 5, 6 pass; 3, 7, 8 to go.
- 2026-10-21T19:27:51.370Z: [claude] 2026-10-21 14:27 George desk QA: item 7 (yes/no card answered by voice) passes after ca967aac. Passing: 1, 2, 4, 5, 6, 7. Left: 3 (dictation pause), 8 (Legacy).
- 2026-10-21T19:56:03.381Z: [claude] 2026-10-21 14:55 George desk QA: items 3 (dictation pause) and 8 (Legacy, including interruptions) pass. All 8 checklist items pass on the dev window at ca967aac.

## Child Tasks
Summary: 23/23 children completed.
- task-203: Calling the desk reaches the team lead, who answers the roster and hands named work straight on (completed)
- task-204: The live voice is the teammate: its identity, the kinds of turn, and acknowledgements that never sound like a hand-off (completed)
- task-206: End to end, docs and the scenario pass: every scenario in the deck proved (completed)
- task-207: The chat gets results, records and one closing note; the pointer only when the chat holds more (completed)
- task-214: S0: the calls live in hotline-voice; hotline-core keeps only the room's side (completed)
- task-223: S5: a call is a thread: hang-up detaches, it goes on as text, a call starts on any thread, one closing note (completed)
- task-224: Proving call after S5: Pork Chop on the QA computer, then George on desk and phone, before S6 (completed)
- task-225: S6: work asked for on a call is done by the teammate's own model on the call's thread, without a word of hand-off (completed)
- task-239: Cards on a call: what needs the person is said on the call, and approvals and simple questions are answered by voice (completed)
- task-240: Interrupting: speech stops at once, what was cut is known, and the teammate never hears itself (completed)
- task-241: Drops: a short drop resumes the same call; a live voice that can't come back says so and offers Legacy, never silently (completed)
- task-242: S5 follow-up: a parked call's typed turns stay in its thread, a long reply at hang-up posts as a summary, and untitled posts read "From the call" (completed)
- task-243: A parked call's row in the chat says it is parked, not "in progress" (completed)
- task-244: The call's thread is its transcript: any agent can read a thread whole, and the home chat acts on "like we said on the call" (completed)
- task-245: Live means live: with a live model selected every call is live, failures are said and never silently become Legacy (completed)
- task-252: S6 follow-up: a live call's transcript is what was said, once; the clone's work shows as work (completed)
- task-254: Live call: a question asked over the voice's speech waits until the person speaks again (completed)
- task-255: Live dictation: a pause inside 'draft… / say… / write…' ends the turn and starts work on half a sentence (completed)
- task-256: Legacy voice: a tool-first turn says nothing for a minute instead of its own first sentence (completed)
- task-258: Phone: every caller turn after a finished reply was recorded as an interruption at 0 ms (completed)
- task-259: One closing note for calls close together, and no repeated reply after hang-up (completed)
- task-260: Legacy: the progress line varies instead of repeating 'I'm still working on that.' (completed)
- task-261: Your name: a Settings field, and the agent can store it when you tell it, with your approval (completed)
