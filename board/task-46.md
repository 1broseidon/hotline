---
id: task-46
title: "Core: build live roster previews from memory"
column: backlog
position: 29
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
updatedAt: "2026-10-21T09:10:53.487Z"
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
assignee: codex
---

## Description
Split from task-19 after source audit on 2026-10-11. Roster updates still reach through tape tails (64 KB → 1 MB → 8 MB) and peer/side streams. Keep preview, open tools and cards in memory from the existing tape broadcast; disk seeds the first snapshot. Send models and slashCommands only when changed. Preserve restart recovery and attention state. Prove equivalent roster output over the real wire, and measure that ordinary tool updates no longer reread tape tails. Coordinate with task-19's fold cache before adding a separate cache.
