---
id: task-22
title: "Window: notify on visible streaming changes and avoid repeated render passes"
parentId: epic-5
column: backlog
position: 14
priority: high
tags:
  - improvement
  - bro-241
createdAt: "2026-10-11T21:05:32.121Z"
description: |-
  Source audit 2026-10-11: tape streaming flushes still notify subscribers, Transcript recomputes groupScheduled/sides, and live rows still get Date.now() timestamps (ui/src/components/Transcript.tsx:645). Compute the visible projection in ThreadStore and notify when it changes; preserve visible thoughts, tool steps and pacing. Use stable live timestamps, memoize derived blocks/activity/stepRuns, gate follow-layout work, and keep the band independent of body updates. Measure render counts and long streaming replies; prove no missing final bubbles or tool/card updates.

  Split the independent former scope into task-49 (long-tape progressive rendering/scroll anchoring) and task-50 (per-desk list caches). CSS :has replacement and solid working-card backgrounds belong here if profiling shows them material.

  Migration source: BRO-241; former BRO-242 and BRO-244 scope is retained in the split tasks.
updatedAt: "2026-10-14T20:05:19.856Z"
---
