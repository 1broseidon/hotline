---
id: task-241
title: "Drops: a short drop resumes the same call; a live voice that can't come back says so and offers Legacy, never silently"
column: in-progress
position: 0
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-20T20:13:09.620Z"
dependsOn:
  - task-223
updatedAt: "2026-10-21T06:30:23.292Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T06:17:32.438Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T06:17:37.452Z"
assignee: codex
decisions:
  - decision-12
---

## Description
Scenarios: A bad network, When the live voice goes away. Decision (George accepted the deck's lean, 2026-10-20): a dropped call is held 20 s before it counts as a hang-up.

Done when:
- A drop shorter than 20 s resumes the same call on the same thread, and the teammate says one line about it.
- A drop longer than 20 s is a hang-up, with everything a hang-up does (S5).
- The phone and the window say Reconnecting only while they really are.
- With no live provider, a spent Voice budget or a failed live connection, the call goes on with Legacy voice and never just ends. One line says what changed. No error codes are spoken; the detail is on the call card.
- A spent budget is said once, with what the person can do about it.
- The relay call drop George hit is diagnosed from the Pi's journal and fixed, or filed with its cause.

Proved by tests with a fake transport for the hold and the fallback, plus a manual relay check on the phone in the scenario pass.

## Log
- 2026-10-21T01:36:55.531Z: [claude] 2026-10-20 George: live means live (task-245). The line 'the call goes on with Legacy voice' is amended: a lost live voice reconnects first, and Legacy is only an offered choice, never automatic.
- 2026-10-21T06:23:16.403Z: [codex] Implemented transport grace and same-device reattachment, immediate pairing revocation, real provider recovery status, and a resume announcement; validating now. task-245 already supplies explicit Legacy offers. Pi host/time and manual phone scenario evidence requested; no phone source lives in this tree.
