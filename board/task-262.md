---
id: task-262
title: Voice nits from the 0.42 scenario pass
column: backlog
position: 33
priority: low
tags:
  - voice
createdAt: "2026-10-21T20:00:50.110Z"
---

## Description
From the Linux scenario pass and George's desk QA, 2026-10-21; none blocked 0.42.
- 'Where are we?' named the wrong three files (it used thread search, not the files' times) and added more than asked.
- After a question card is announced, the voice repeats the ask in its own words ('The card is up. Say yes or no…').
- The first words of a desk call's first line get clipped ('… team' for 'Who is on the team?').
- The clone reused qa.sonnet.txt for a new sonnet and overwrote the earlier one without being asked; files made on a call should not silently replace an earlier file of the same name.
- task-255's unfinished-dictation check is English-only.
- Grok keeps 'say this' items in its conversation (it cannot delete items); a prompt rule now forbids reading them back, which lowers but doesn't prove away the leak George heard at 13:11.
