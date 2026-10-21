---
id: task-258
title: "Phone: every caller turn after a finished reply was recorded as an interruption at 0 ms"
column: backlog
position: 33
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T13:51:57.917Z"
updatedAt: "2026-10-21T14:26:37.143Z"
---

## Description
George's desk QA, 2026-10-21 08:39 (call 6952a288, Grok Voice, dev window on 0d7c2da2): each time he spoke after Goldie had finished a line, the call recorded 'Speech interrupted. Playback stopped at 0 ms. The caller did not hear this line to the end: …' — for 'Good morning…', 'Not yet…', 'Still writing…'. Those lines were heard. The cut notice goes into the model's context, so it is told the caller missed lines they heard. Not seen on the Linux QA desk, where a turn after a finished reply records no cut. Suspect: on the Mac window the player still reads as playing (or a stale audio_item) when the reply has finished, so the next speech_started truncates at playedMs 0. Done when: speaking after a reply has finished records no interruption on the Mac desk; a real barge still records the played ms.

## Log
- 2026-10-21T14:26:37.688Z: [claude] 2026-10-21 Pork Chop: not the Mac — both calls were from George's phone (client=phone). The phone's frontStop handler barges unconditionally, and the desk sends frontStop at every caller turn, so each turn after a finished reply reported 0 ms played. Desk fix: front_barge with 0 ms while the front isn't responding cuts nothing (test a_barge_after_a_finished_reply_records_no_cut fails without it). The phone should also only barge while it is playing (hotline-mobile follow-up).
- 2026-10-21T20:00:35.482Z: [claude] 2026-10-21 Pork Chop: shipped on release/0.42.0 in 7f47de0c (desk ignores a 0 ms barge while the voice isn't generating), with test a_barge_after_a_finished_reply_records_no_cut. The phone-side cleanup is hotline-mobile's own task (barge only while playing).
