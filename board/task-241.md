---
id: task-241
title: "Drops and fallbacks: a short drop resumes the same call, and a lost live voice carries on as Legacy voice"
column: backlog
position: 46
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-20T20:13:09.620Z"
dependsOn:
  - task-223
updatedAt: "2026-10-20T20:13:25.588Z"
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
