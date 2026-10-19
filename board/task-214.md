---
id: task-214
title: The calls move into hotline-voice behind the approved interface
column: todo
position: 11
priority: high
tags:
  - crate-split
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-19
createdAt: "2026-10-19T17:20:19.714Z"
---

## Description
Only after George approves the design (task above). Move the call code into hotline-voice behind the interface it settles; the room implements it in the core, the voice harness tests and test rooms implement it in tests. hotline-core keeps no voice module beyond the glue that hands the room to the calls. Desk and phone calls, the live fronts (Grok Voice, GPT Realtime, GPT-Live) and Legacy voice behave exactly as before: the desk harness's call tests and processes/voice tests pass unchanged. Timings re-run into ADR 0007.
