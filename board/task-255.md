---
id: task-255
title: "Live dictation: a pause inside 'draft… / say… / write…' ends the turn and starts work on half a sentence"
column: backlog
position: 40
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T08:37:50.053Z"
---

## Description
Linux QA, 2026-10-21, Grok Voice, 2735d01a: 'Draft a message to the harbour master saying that' + 3.5 s pause + 'the fog horn should be tested on Friday morning, before the boats go out.' The provider's VAD ended the turn at the pause; the front called work_on_it with the half sentence and guessed content ('saying that: the harbor note…'), then the second half steered it. The prompt's remain_silent rule for unfinished phrases isn't enough: the turn boundary is the provider's. Done when: a pause of up to ~4 s inside an unfinished dictation phrase doesn't start work (e.g. a longer server-VAD silence window while a dictation verb is open, or holding work_on_it until the phrase completes), on Grok Voice and GPT Realtime, with a scripted-front test.
