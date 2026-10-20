---
id: task-240
title: "Interrupting: speech stops at once, what was cut is known, and the teammate never hears itself"
column: backlog
position: 45
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-20T20:13:09.138Z"
dependsOn:
  - task-223
updatedAt: "2026-10-20T20:13:25.135Z"
---

## Description
Scenario: Interrupting.

Done when:
- Speech stops the moment the person starts talking or taps, on the desk and the phone, on both live voices and Legacy voice.
- What was cut is recorded as cut on the call's thread, so the teammate knows what the person did not hear. The next reply answers the interruption, not the rest of the old one.
- On speaker the teammate never hears itself: echo cancellation covers its own speech on the desk and the phone. It is unverified today, so it is checked by hand in the scenario pass (task-206) and fixed if it fails.

Proved by tests over the turn and record code for the cut line, plus the manual checks.
