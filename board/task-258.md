---
id: task-258
title: "Mac desk: every caller turn after a finished reply is recorded as an interruption at 0 ms"
column: backlog
position: 33
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T13:51:57.917Z"
---

## Description
George's desk QA, 2026-10-21 08:39 (call 6952a288, Grok Voice, dev window on 0d7c2da2): each time he spoke after Goldie had finished a line, the call recorded 'Speech interrupted. Playback stopped at 0 ms. The caller did not hear this line to the end: …' — for 'Good morning…', 'Not yet…', 'Still writing…'. Those lines were heard. The cut notice goes into the model's context, so it is told the caller missed lines they heard. Not seen on the Linux QA desk, where a turn after a finished reply records no cut. Suspect: on the Mac window the player still reads as playing (or a stale audio_item) when the reply has finished, so the next speech_started truncates at playedMs 0. Done when: speaking after a reply has finished records no interruption on the Mac desk; a real barge still records the played ms.
