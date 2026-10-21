---
id: task-256
title: "Legacy voice: a tool-first turn says nothing for a minute instead of its own first sentence"
priority: medium
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T08:37:51.638Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T08:38:48.810Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T08:38:50.424Z"
    ended: "2026-10-21T08:54:00.707Z"
    outcome: done
assignee: codex
updatedAt: "2026-10-21T08:54:00.707Z"
decisions:
  - decision-17
completedAt: "2026-10-21T08:54:00.273Z"
---

## Description
Linux QA, 2026-10-21, Legacy voice, Rex on Grok 4.7, a180a43d: 'Search the web for how tall the Bell Rock lighthouse is' went straight to ~20 web_search calls with no spoken first sentence for over a minute (the transcription misheard the name, which made it long). The voice contract asks for a short spoken sentence before a tool; this model ignored it. Whether the one soft cue played can't be heard on the QA computer. Done when: on Legacy, a turn whose first output is a tool call gets the agent's own short sentence or the single soft cue within a second or two, and a long tool run gets an occasional short progress line rather than a minute of silence; checked with Hotline Agent on Grok and an ACP agent.

## Log
- 2026-10-21T08:43:35.038Z: [codex] Found that Legacy suppresses its 1.8s cue on Said before TTS yields audio, and has no progress speech after the cue. Fix tracks actual clips on Tokio time and queues guarded 20s quiet-turn progress; adding paused-clock and real-wire Hotline Agent/ACP fixture regressions on dispatch/task-256.
- 2026-10-21T08:51:25.099Z: [codex] Focused proof passes: 15 direct-call tests plus the real-wire tool-first regression on native xAI/Grok 4.7 Responses fixtures and the ACP child. Single cue precedes progress clips; two quiet periods speak progress before the final answer. Decision-17 supersedes decision-13. Full make check and precheck running; physical Linux/phone audibility and live-provider QA remain reviewer checks.
- 2026-10-21T08:53:49.773Z: [codex] Ready for review on dispatch/task-256, commit bff52cb2. Legacy now gates its single 1.8s cue on actual audio and queues turn-checked progress after 20 quiet seconds; delayed TTS, completion/hold, native Grok 4.7 Responses fixtures and the real ACP child are covered. make check and all 8 make precheck legs pass; worktree clean. Next: review the branch and verify live Grok plus physical Linux/phone playback audibility; fixture clips establish wire delivery, not what a speaker played.
- 2026-10-21T08:53:59.562Z: Landed on release/0.42.0
