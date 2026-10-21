---
id: task-19
title: "Core: cache folded streams and chapter activity on hot paths"
parentId: epic-5
column: in-progress
position: 0
priority: high
tags:
  - improvement
  - bro-235
createdAt: "2026-10-11T21:05:29.635Z"
description: |-
  Source audit 2026-10-11: Log::load still reads and folds every readable file (crates/hotline-core/src/log/mod.rs:128), and prompt/page/card paths repeatedly load full tapes. Own a room-always/tape-bounded fold cache in Log, kept coherent by append and migration; alternatively use indexed tail/single-event reads if measurements justify them. Keep open chapter id and activity in memory rather than reloading for every prompt. Preserve folded event revisions, segment compatibility and one writer per stream. Prove cold/warm reads, append revisions, compaction/migration and restart through existing log tests and real wire snapshots; measure representative 29 MB tapes.

  Split the independent former scope into task-46 (roster projections), task-47 (list reads), and task-48 (write locks/indexing). Those tasks remain children of epic-5 and should reuse this cache instead of inventing competing ones.

  Migration source: BRO-235, with BRO-237/239/240 now tracked by the split tasks.
updatedAt: "2026-10-21T09:11:12.100Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T09:10:48.958Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T09:10:50.863Z"
assignee: codex
---

## Log
- 2026-10-21T09:11:25.982Z: [codex] task-46 is auditing wire/roster.rs and store/previews.rs now. Please keep shared fold-cache APIs in Log; roster will own derived preview/open-tool/card projection only, seeded through Log and updated from broadcasts. Please note intended cache API/invalidation here so we avoid duplicate folded-event caches.
- 2026-10-21T09:12:48.271Z: [codex] task-46 needs atomic Log snapshot+subscribe to seed derived roster/card projections without replaying older buffered revisions over the seed. Please expose pub fn snapshot(&self, stream: &StreamId) -> (Vec<Value>, broadcast::Receiver<Value>) under writer lock, using fold cache. I will add this small method in log.rs for my branch; expected merge overlap limited to that method. Derived projection will retain only preview, current tool, pending card ids, not folded event history.
