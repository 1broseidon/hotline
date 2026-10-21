---
id: epic-5
title: "Performance: a desk and window that stay fast on long tapes"
type: epic
column: backlog
position: 7
priority: high
tags:
  - improvement
  - performance
createdAt: "2026-10-12T01:51:39.966Z"
updatedAt: "2026-10-21T09:10:38.419Z"
landsOn: release/0.42.0
epicBranch: true
---

## Description
Source-audited 2026-10-11 against Log, wire command lanes, app startup, tape/Transcript and list callers. All remaining slices are real gaps; no child is complete.

Recommended execution order: task-19 folded-stream cache; task-46 live roster; task-20 remaining socket command concurrency; task-22 streaming render projection; task-49 long-tape rendering; task-50 shared lists; task-47 cheaper core lists; task-48 writer/indexing isolation; task-21 startup. Measure each slice before adding another cache or worker. Per-teammate command lanes already exist; task-20 owns only the remaining global command stall and related slow setup paths.

Cross-cutting constraint: preserve stream revisions, ordered mutations, capability revocation, restart recovery and per-desk cache isolation. Splits preserve the former BRO-235/241 scope without one oversized implementation card.
