---
id: task-241
title: "Drops: a short drop resumes the same call; a live voice that can't come back says so and offers Legacy, never silently"
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-20T20:13:09.620Z"
dependsOn:
  - task-223
updatedAt: "2026-10-21T06:40:37.970Z"
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
    ended: "2026-10-21T06:40:37.970Z"
    outcome: done
assignee: codex
decisions:
  - decision-12
completedAt: "2026-10-21T06:40:37.472Z"
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
- 2026-10-21T06:34:45.403Z: [codex] Committed 9345dcf9 on dispatch/task-241. Core holds transport loss 20 s, resumes same call/thread for the same device, ends immediately on pairing revocation, and expires through S5. Window waits for the resumed snapshot; provider recovery reports real status. Resume line and fixed local live-failure/budget notices preserve explicit Legacy choice. Clock-controlled tests, real wire allow/deny and reconnect tests, UI tests, make check and all 8 precheck legs passed; final rerun underway after the last interruption-context/test refinements. Remaining: no Pi host/incident time supplied, so no journal diagnosis or cause claimed; phone source/adoption and physical relay scenario are outside this tree and unverified. Pre-start refusals retain task-245's call-card explanation; bundled audible notices apply to an existing call.
- 2026-10-21T06:37:50.091Z: [codex] Final rerun hit ENOSPC in two unrelated ACP post_to_chat integration tests (their traces explicitly report No space left on device writing AGENTS.md/job streams). Removed only this worktree's disposable incremental and Windows build artifacts; rerunning make check. The preceding full check and final precheck passed; branch remains clean.
- 2026-10-21T06:40:21.533Z: [codex] Final verification on committed 9345dcf9: make check passed after removing disposable build artifacts (both ENOSPC-hit ACP tests pass); make precheck passed all 8 legs including Windows. dispatch/task-241 is clean and ready for code review. Next action: obtain Pi SSH host and incident time, diagnose the journal drop, wire/verify phone adoption of the 20 s resume contract and reconnecting state, then perform the physical relay scenario. Those phone/Pi acceptance items are not claimed complete.
- 2026-10-21T06:40:36.663Z: Landed on release/0.42.0
