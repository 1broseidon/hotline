---
id: task-261
title: "Your name: a Settings field, and the agent can store it when you tell it, with your approval"
column: in-progress
position: 0
priority: medium
tags:
  - voice
  - settings
parentId: epic-21
createdAt: "2026-10-21T19:59:26.951Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T19:59:35.875Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T19:59:40.694Z"
assignee: codex
updatedAt: "2026-10-21T20:04:16.290Z"
decisions:
  - decision-29
---

## Description
George, 2026-10-21: the voice reads the personName setting (task-204) but nothing sets it. Done when:
- Settings → General has a 'Your name' field (short label per ui decisions; tooltips 2–3 words) that reads and writes personName.
- Agents have a tool to set the person's name when the person tells them ('call me George'). Setting or changing it raises an approval card ('Save your name as George?'); only an approved answer writes personName. On a call this card is answerable by voice (it is a yes/no question).
- The live voice and the home agent use the name once set (already wired for the voice via personName).
- docs/security.md: the tool can only change personName, only with the person's approval.
- Tests: the tool raises an approval card; approve → personName set; decline → unchanged; Settings field round-trips.
Lands on release/0.42.0. Merge release/0.42.0 first, make check in the foreground, commit before done.

## Log
- 2026-10-21T20:00:48.348Z: [codex] Started on dispatch/task-261. release/0.42.0 is already contained in HEAD (zero commits to merge). Reusing core-owned question cards for a narrowly bound name approval; ordinary question prose will never authorize a settings write.
