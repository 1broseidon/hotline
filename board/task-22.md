---
id: task-22
title: "Window: notify on visible streaming changes and avoid repeated render passes"
parentId: epic-5
column: in-progress
position: 1
priority: high
tags:
  - improvement
  - bro-241
createdAt: "2026-10-11T21:05:32.121Z"
description: |-
  Source audit 2026-10-11: tape streaming flushes still notify subscribers, Transcript recomputes groupScheduled/sides, and live rows still get Date.now() timestamps (ui/src/components/Transcript.tsx:645). Compute the visible projection in ThreadStore and notify when it changes; preserve visible thoughts, tool steps and pacing. Use stable live timestamps, memoize derived blocks/activity/stepRuns, gate follow-layout work, and keep the band independent of body updates. Measure render counts and long streaming replies; prove no missing final bubbles or tool/card updates.

  Split the independent former scope into task-49 (long-tape progressive rendering/scroll anchoring) and task-50 (per-desk list caches). CSS :has replacement and solid working-card backgrounds belong here if profiling shows them material.

  Migration source: BRO-241; former BRO-242 and BRO-244 scope is retained in the split tasks.
updatedAt: "2026-10-21T10:27:29.838Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T10:22:50.369Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T10:22:54.509Z"
assignee: codex
decisions:
  - decision-21
---

## Log
- 2026-10-21T10:22:49.324Z: [claude] 2026-10-21 Pork Chop: lands on dispatch/epic-5 (now 3891214f, includes task-19's writer-held fold cache and task-46). Before done: merge dispatch/epic-5 into your branch, run make check in the foreground on the merged result, commit, and show a before/after measurement.
- 2026-10-21T10:27:29.496Z: [codex] Implemented visible streaming projection, atomic queued-word settlement, stable live stamps, memoized transcript/work derivations, isolated conversation/work bands, and gated follow layout. Synthetic 60,000-character / 1,000-frame Transcript fixture: baseline 1,000 notifications/renders and 170.9 ms; changed 1 notification/render and 56.4 ms, final DOM bubble retained. Running required make check and precheck on dispatch/epic-5 base 0a1127a0.
- 2026-10-21T10:36:36.286Z: [codex] Finished on dispatch/task-22, commit dd0a6254; clean worktree. Merged/confirmed dispatch/epic-5 (0a1127a0), ran make check in foreground on the final result, and passed all 8 make precheck legs. Visible projection/atomic settlement, stable live stamps with cadence, memoized derivations, independent bands, gated follow layout; tests cover final DOM bubbles and live tool/card changes. Before/after 60k-character, 1000-frame fixture: 1000→1 notifications and transcript-reader renders, 170.9→56.4 ms (synthetic timing). Ready for review.
