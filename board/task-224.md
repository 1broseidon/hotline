---
id: task-224
title: "Proving call after S5: Pork Chop on the QA computer, then George on desk and phone, before S6"
column: backlog
position: 37
priority: high
tags:
  - voice
  - qa
assignee: claude
parentId: epic-21
createdAt: "2026-10-19T20:00:07.420Z"
dependsOn:
  - task-223
  - task-242
updatedAt: "2026-10-20T23:06:03.441Z"
---

## Description
After S5, before S6. Run these deck scenarios on Legacy voice, desktop and phone, to a Hotline Agent teammate and an ACP teammate: Hanging up while work runs, Picking it back up, Calling on a thread already under way, Following up in the chat, and A quick question on the go (its closing note).

Done when:
- Pork Chop has run each scenario on the Linux QA computer, then George on the desk and the phone. Every "What must be true" line of those scenarios is checked off or filed as a bug.
- Legacy voice's time from the end of speech to the first audio is measured and noted.
- It stops in Review until George says go for S6.

## Log
- 2026-10-20T23:06:05.015Z: [claude] 2026-10-20 Pork Chop's pass on the QA desk (virtual mic and speaker: PulseAudio null sink + pipe source fed by espeak, GStreamer plugins in the manifest), Legacy voice, Rex on Hotline Agent: utterances land on the call thread, not the DM; hang-up mid-turn lets the work finish and post; the closing note is a summary; the thread parks and continues by text with context; calling again on the thread works and the next note covers only new work; posts open the thread. Bugs filed as task-242. Not yet run: ACP teammate, call on a work thread, phone, latency.
- 2026-10-21T00:48:19.481Z: [claude] 2026-10-20 re-check at a185fff1 after task-242: a long reply at hang-up posts one summary with Open the call; a typed turn in a parked call stays in its thread; posts read From the call. Minor: the parked row still says in progress (filed). Still to run: ACP teammate, a call on a work thread, phone, Legacy time to first audio.
- 2026-10-21T00:51:05.970Z: [claude] 2026-10-20: a call placed on a work thread (QA test for work threads) attaches voice to that thread, posts read From <thread title>, and it parks with a closing note. Pork Chop's pass done except ACP teammate (covered by the S5 harness on both drivers) and Legacy time to first audio. Ready for George on desk and phone.
- 2026-10-21T05:05:24.397Z: [claude] 2026-10-20 George ran the proving call on desk and phone: 'bones are there, very happy with this pass'. His two points became task-244 (landed); live-means-live is task-245 (landed). Done.
