---
id: task-245
title: "Live means live: with a live model selected every call is live, failures are said and never silently become Legacy"
column: in-progress
position: 1
priority: high
tags:
  - voice
parentId: epic-21
dependsOn:
  - task-214
createdAt: "2026-10-21T01:36:48.450Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T01:36:56.372Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T01:36:57.472Z"
assignee: codex
updatedAt: "2026-10-21T01:37:18.022Z"
---

## Description
George, 2026-10-20 20:34: "I feel like it flip flops often. I want live to always use live when selected in settings." This supersedes the deck's lean "Legacy voice ... is the fallback" (task-241's fallback line is amended to match).

What the code does today: each call asks voice.status whether the desk offers a front ("voiceFront", from front_available()). If it doesn't, or call_start gives no front, the window (ui/src/voice/call.ts) and the phone quietly place a Legacy call. Mid-call, a lost front (open fails, events end or error, or front_connected false at a minute tick) also goes on as Legacy (duplex.rs lose_front). The dev desk log has no "front could not open" or "the front failed" lines, so George's flip-flops come from the start-of-call decision. Which condition said no is not logged. The six "the front refused a request: responses_validation_failed / invalid_request_error" lines are a separate fault: a request the desk sends Grok Voice is rejected, and that refusal is not logged in enough detail to fix.

Done when:
- With a live model selected in Settings → Live calls, every call is live, from the window, the hotkey and the phone, on any thread. A call never quietly becomes Legacy.
- If live voice can't start, the call doesn't start as Legacy. It says, in one sentence on the call card, why (no xAI sign-in, budget, provider unreachable), and offers "Call with Legacy voice" as an explicit choice.
- If live voice drops mid-call, the desk reconnects it with the call's context (as the hourly renewal already does). Only if it can't, the call says so once and offers to carry on with Legacy voice. It never switches silently.
- Every start-of-call and mid-call decision about live voice is logged with its reason, so a flip-flop can be traced from the log.
- Each Grok Voice refusal logs the rejected event's type and the provider's message (no secrets). The request behind George's six refusals is found and fixed.
- Legacy voice is used only when it is the selected model, or when the person picks it after a failure.

Proved by desk-harness tests with a fake realtime provider for each line (front unavailable, open fails, drop then reconnect, drop with no reconnect), a UI test that the window never places a Legacy call when live is selected, and a manual check in the scenario pass on the desk and the phone.
