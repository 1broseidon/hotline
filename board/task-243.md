---
id: task-243
title: A parked call's row in the chat says it is parked, not "in progress"
column: in-progress
position: 0
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
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T05:05:39.270Z"
assignee: codex
updatedAt: "2026-10-21T05:05:56.709Z"
---

## Description
Found re-checking task-242 on the QA desk, 2026-10-20, at a185fff1. After hang-up, once the closing note was posted and the call's thread pane said "Parked: no agent is running", the call's row in the chat still read "<title> · in progress · Open". Done when: the row reads parked (as the pane does) once the thread parks, and running only while a turn is in flight; a desk-harness or UI test proves both states.

## Log
- 2026-10-21T05:07:10.271Z: [codex] Found callWords treating every non-closed link as in progress. The saved live state means open, not a turn in flight; label live calls open and parked calls parked, retaining closed-call duration/outcome. UI regression covers both states.
- 2026-10-21T05:09:35.230Z: [codex] Extended fix to publish workingCalls in the roster, using the core's actual per-call turn state. Chat rows now distinguish running/open/parked; the pane refreshes from the same roster signal. UI regression passes, including rerenders and Open; extending existing real-core hang-up harness to prove roster start/finish for both drivers.
