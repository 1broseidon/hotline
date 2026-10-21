---
id: decision-10
title: A live call records speech once by id; the clone's answer is work, not speech.
type: decision
status: current
claimKey: voice.call-transcript
kind: decision
tags:
  - voice
createdAt: "2026-10-21T06:01:23.000Z"
---

## Description
A live call records speech once by id; the clone's answer is work, not speech.
Live text is buffered at response boundaries and folds by speech id when interrupted or flushed before work. Clone prose never streams as speech or agent bubbles; tool entries remain and the completed answer is one tool entry with toolKind work_result, also included in search and closing summaries. Only the final answer after the last tool is offered to live voice, so the clone's opening cannot become a second acknowledgement. Existing transcript shapes and tape layout remain compatible.
