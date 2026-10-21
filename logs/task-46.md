---
id: task-46
title: "Core: build live roster previews from memory"
description: Split from task-19 after source audit on 2026-10-11. Roster updates still reach through tape tails (64 KB → 1 MB → 8 MB) and peer/side streams. Keep preview, open tools and cards in memory from the existing tape broadcast; disk seeds the first snapshot. Send models and slashCommands only when changed. Preserve restart recovery and attention state. Prove equivalent roster output over the real wire, and measure that ordinary tool updates no longer reread tape tails. Coordinate with task-19's fold cache before adding a separate cache.
priority: high
tags:
  - performance
  - core
relatedFiles:
  - crates/hotline-core/src/wire/roster.rs
  - crates/hotline-core/src/store/previews.rs
parentId: epic-5
createdAt: "2026-10-12T04:20:47.697Z"
updatedAt: "2026-10-21T09:32:08.118Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T09:10:49.219Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T09:10:53.487Z"
    ended: "2026-10-21T09:32:08.118Z"
    outcome: done
assignee: codex
decisions:
  - decision-20
completedAt: "2026-10-21T09:32:07.404Z"
---

## Description
Split from task-19 after source audit on 2026-10-11. Roster updates still reach through tape tails (64 KB → 1 MB → 8 MB) and peer/side streams. Keep preview, open tools and cards in memory from the existing tape broadcast; disk seeds the first snapshot. Send models and slashCommands only when changed. Preserve restart recovery and attention state. Prove equivalent roster output over the real wire, and measure that ordinary tool updates no longer reread tape tails. Coordinate with task-19's fold cache before adding a separate cache.

## Log
- 2026-10-21T09:24:40.935Z: [codex] Implemented broadcast-fed tape and live side/peer card projections, atomic Log seed/subscription, and negotiated rosterOptions catalog deltas. Coordination with concurrent task-19 is in its log: snapshot_with must use its already-locked fold reader. Explicit MutexGuard<()> type makes that integration fail compilation rather than silently recursively locking the cache. Added long-tape real-wire/restart/lag/catalog tests and per-directory tail-read instrumentation; running full gates.
- 2026-10-21T09:27:31.992Z: [codex] Measured debug real-wire run: ~29 MB tape, cold roster seed 90.3 ms, 100 acknowledged tool rows 22.9 ms, zero live tape-tail reads (former preview path needs two tail windows for each unchanged message). Full-row restart equality, tape lag, existing attention tests and catalog delta tests pass. Full gates still running; updated phone hello expected-capability fixture for rosterOptions.
- 2026-10-21T09:30:40.149Z: [codex] make check passes. task-19 landed as d722b8fe on dispatch/epic-5 while this task was running. On integration, implement snapshot_with as self.with_fold(stream, false, |fold| (project(fold.events.clone()), self.subscribe(stream))).expect("a tolerant load skips unreadable files"); with_fold holds the cache/writer guard through the callback. Remove the old MutexGuard<()> body. Branch instructions require a task-log request or explicit approval before I merge epic-5; permission requested, not yet granted.
- 2026-10-21T09:31:51.340Z: [codex] Implementation committed on dispatch/task-46 as 59f824c4; worktree clean. make check and all eight make precheck legs pass; measured zero live tail reads across 100 tool updates. Task-19 has since landed on epic-5, so the snapshot_with writer boundary must be reconciled before landing this branch. Awaiting explicit permission or the required conflict-resolution log request to merge epic-5; leaving task In Progress until that integration is verified, then move to Review.
- 2026-10-21T09:32:06.750Z: Landed on dispatch/epic-5
- 2026-10-21T10:18:26.830Z: [claude] 2026-10-21 Pork Chop: the landed epic branch (71ba6c03) did not compile: snapshot_with typed the writer guard as MutexGuard<()> but task-19 made it Mutex<Cache>, and load() takes that lock, so a type-only fix would deadlock. Fixed on dispatch/epic-5 with with_fold_in on the held guard (store 94/94, roster 23/23); make check running. The landing let a non-compiling merge through — the hand-off validated its own branch before the epic merge, not after.
