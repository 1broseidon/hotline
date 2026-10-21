---
id: task-259
title: One closing note for calls close together, and no repeated reply after hang-up
column: backlog
position: 34
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T14:10:52.512Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T14:10:53.993Z"
  until: done
  tries: 0
assignee: codex
updatedAt: "2026-10-21T14:10:54.121Z"
---

## Description
George's desk QA, 2026-10-21 08:39–09:00: three calls to Goldie within a few minutes, and the chat filled with summaries. The 08:57 call alone posted three: the work result with qa.sonnet.txt (right); then the voice's own reply that landed after hang-up ('Done. The new sonnet is saved… I can't open it on your phone, so the file is the one to open.') repeating it, and wrong (he was at the desk); then the closing note repeating both and claiming 'The work summary was cut off, so check the file for the full text.'

George, 09:10: wait a little longer before summarising, so picking up and hanging up a few times in a few minutes doesn't produce three summaries in a row.

Done when:
- The closing note waits 2 minutes after hang-up (one named constant). A new call to the same teammate inside that window cancels the pending note; when the last call in the run settles, one note covers every call in it and links each.
- A reply that lands after hang-up is posted only if it carries something the chat doesn't already have from this call (e.g. not when the work result or file was already posted); otherwise the closing note covers it.
- The closing note never claims something was cut off when the result is in the chat.
- Tests (paused clock): two calls 60 s apart → one note covering both; two calls 3 min apart → two notes; a work result posted before hang-up plus a late reply → result and one note, no repeat.
Lands on release/0.42.0. Merge release/0.42.0, make check in the foreground, commit before done.
