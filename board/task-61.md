---
id: task-61
title: "Desk: call wire for front calls"
column: todo
position: 41
priority: high
tags:
  - voice
  - calls
  - wire
  - 0.37.0
parentId: epic-6
dependsOn:
  - task-58
createdAt: "2026-10-14T20:03:57.847Z"
---

## Description
Negotiate a front call: capability in hello and voice.status (e.g. voiceFront), voice.call_start mode, echoed so an older desk/phone falls back to the current pipeline. Full-duplex PCM both ways over the sealed wire at the provider's rate (Grok likely 24 kHz; resample on the desk if the phone sends 16 kHz), small chunks, sequence numbers, backpressure. Barge-in: phone reports speech start / playback position; desk truncates the front's response. Call states (listening, speaking, teammate working) for the call card and island. Owner/local seat enforcement unchanged.
