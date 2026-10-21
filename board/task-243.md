---
id: task-243
title: A parked call's row in the chat says it is parked, not "in progress"
column: backlog
position: 45
priority: medium
tags:
  - voice
  - ui
parentId: epic-21
createdAt: "2026-10-21T00:48:18.956Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T05:05:35.831Z"
  until: done
  tries: 0
assignee: codex
updatedAt: "2026-10-21T05:05:35.962Z"
---

## Description
Found re-checking task-242 on the QA desk, 2026-10-20, at a185fff1. After hang-up, once the closing note was posted and the call's thread pane said "Parked: no agent is running", the call's row in the chat still read "<title> · in progress · Open". Done when: the row reads parked (as the pane does) once the thread parks, and running only while a turn is in flight; a desk-harness or UI test proves both states.
