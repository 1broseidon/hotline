---
id: task-252
title: "S6 follow-up: a live call's transcript is what was said, once; the clone's work shows as work"
column: in-progress
position: 0
priority: high
tags:
  - voice
  - bug
parentId: epic-21
dependsOn:
  - task-225
createdAt: "2026-10-21T05:51:57.076Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T05:51:57.833Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T05:51:59.039Z"
assignee: codex
updatedAt: "2026-10-21T05:52:17.189Z"
---

## Description
Found in Pork Chop's QA on the Linux QA desk, 2026-10-21 ~01:00, at a23dc243. Real Grok Voice call to Rex (Hotline Agent, Grok 4.7): "write a three line poem … save it in a file called crane dot text". The work itself went right: the live voice connected (grok-voice-latest), work_on_it ran a clone, crane.txt was written, the poem was posted, a closing note followed, and the voice used no hand-off words. But the call's thread (calls/93f94540….jsonl) shows:

1. Each line the live voice said is written several times as it streams, under the same id. "Done. The poem about farmer Crane is" / "…saved in crane.txt. The full text is in the" / "…in the chat." are three agent events with id front-…-2, and every line of the poem is similar. The thread view shows them as repeated lines, and read_thread will return them all.
2. The clone's own reply ("Saving the poem to `crane.txt`.", then "Saved in `crane.txt`. Farmer Crane …" and the poem's paragraphs as -2, -3, -4) is written to the call thread as agent lines, so the transcript reads as if the teammate said the poem twice: once as the clone's prose, and once as the voice reading it aloud.
3. The voice gave two acknowledgements for one request: "Writing that poem to crane dot text." and then the clone's "Saving the poem to crane dot text.".
4. The voice read the whole poem aloud after saying "The full text is in the chat." (task-207 owns the pointer rule; noted here so the transcript fix doesn't hide it).
5. Settings → Live calls → Grok Voice still describes it as "A full agent: talks with you, looks things up and hands heavy work to your teammate", which is the hand-off framing George asked to remove.

Done when:
- Each line the live voice says is recorded on the call's thread once, as its final text. Streaming updates fold into one line by id, in the stream or before they are written, and read_thread returns one line per thing said.
- The clone's work appears on the call's thread as work: its tool calls, and one result line marked as the work's result. It never appears as lines the teammate said on the call. What the voice says about the result is recorded once, as said.
- One request gets at most one spoken acknowledgement. The clone's own opening line is never spoken after the voice's.
- The Settings description of a live model says the voice is the teammate (e.g. "Talks with you as your teammate, looks things up, and does real work on your teammate's own model"), with no hand-off wording. The UI copy test covers it.

Proved by desk-harness tests with the fake realtime provider (streamed updates fold to one line; the clone's result is a work line, not speech; one acknowledgement), the thread view's UI test, and a re-run of this call on the QA desk.
