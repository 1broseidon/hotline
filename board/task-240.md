---
id: task-240
title: "Interrupting: speech stops at once, what was cut is known, and the teammate never hears itself"
column: in-progress
position: 1
priority: high
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-20T20:13:09.138Z"
dependsOn:
  - task-223
updatedAt: "2026-10-21T05:17:40.843Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T05:05:36.109Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T05:05:41.124Z"
assignee: codex
decisions:
  - decision-9
---

## Description
Scenario: Interrupting.

Done when:
- Speech stops the moment the person starts talking or taps, on the desk and the phone, on both live voices and Legacy voice.
- What was cut is recorded as cut on the call's thread, so the teammate knows what the person did not hear. The next reply answers the interruption, not the rest of the old one.
- On speaker the teammate never hears itself: echo cancellation covers its own speech on the desk and the phone. It is unverified today, so it is checked by hand in the scenario pass (task-206) and fixed if it fails.

Proved by tests over the turn and record code for the cut line, plus the manual checks.

## Log
- 2026-10-21T05:10:57.035Z: [codex] Found and fixing Legacy's playback-time microphone gate and live's discarded cut transcript. Desk barge-in now stops locally at confirmed speech onset; core records interrupted generated text without guessing the last audible word, prevents resumed streamed tails, and supplies playback context to the next thread turn separately from the caller's words. Phone client is outside this worktree; its onset wiring and physical speaker/AEC checks remain for task-206. Validation in progress.
- 2026-10-21T05:19:06.283Z: [codex] Committed 2a4c599b on dispatch/task-240. UI suite 474/474, hotline-voice 163 passed (3 existing ignored), and focused cut-turn regression pass. make precheck passed all 8 legs, including Windows and isolated UI files. Full make check is building/running the workspace tests; wire cut-record/next-turn and live-tail scenarios are included. decision-9 records that cut generation is not an exact heard transcript. Manual desk/phone speaker matrix is explicitly handed to task-206.
