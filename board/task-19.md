---
id: task-19
title: "Core: cache folded streams and chapter activity on hot paths"
parentId: epic-5
column: backlog
position: 11
priority: high
tags:
  - improvement
  - bro-235
createdAt: "2026-10-11T21:05:29.635Z"
description: |-
  Source audit 2026-10-11: Log::load still reads and folds every readable file (crates/hotline-core/src/log/mod.rs:128), and prompt/page/card paths repeatedly load full tapes. Own a room-always/tape-bounded fold cache in Log, kept coherent by append and migration; alternatively use indexed tail/single-event reads if measurements justify them. Keep open chapter id and activity in memory rather than reloading for every prompt. Preserve folded event revisions, segment compatibility and one writer per stream. Prove cold/warm reads, append revisions, compaction/migration and restart through existing log tests and real wire snapshots; measure representative 29 MB tapes.

  Split the independent former scope into task-46 (roster projections), task-47 (list reads), and task-48 (write locks/indexing). Those tasks remain children of epic-5 and should reuse this cache instead of inventing competing ones.

  Migration source: BRO-235, with BRO-237/239/240 now tracked by the split tasks.
updatedAt: "2026-10-21T09:10:49.085Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T09:10:48.958Z"
  until: done
  tries: 0
assignee: codex
---
