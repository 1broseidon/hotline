---
id: task-187
title: Run read-only global commands without blocking the socket
column: review
position: 13
priority: high
tags:
  - wire
  - performance
parentId: epic-14
createdAt: "2026-10-18T23:40:56.843Z"
relatedFiles:
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/tests.rs
updatedAt: "2026-10-19T00:03:20.470Z"
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/src/wire/tests.rs
      description: a slow read does not hold up the next command; mutating commands stay in order
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-18T23:54:34.013Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:03:10.606Z"
    duration: 517
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-18T23:54:15.490Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-18T23:54:20.975Z"
    ended: "2026-10-19T00:03:20.470Z"
    outcome: review
assignee: claude
---

## Description
Slice of task-20. wire/socket.rs: per-teammate lanes exist (:231), but other commands are awaited inline (:244-269). Spawn the non-mutating global reads (models.list, search, voice.status and similar), as file reads already are (:445-464). Keep every mutating command, settings.update included, in order. Test that a slow read doesn't hold up the next command.

Split from task-20, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:03:13.966Z: [claude] Done on branch dispatch/task-187 (002103b6). wire/socket.rs command(): the existing file.read/avatar.read spawn arm now also spawns voice.status, voice.models, images.status, websearch.status, capabilities.options, remote.status, remote.relays, desks.links, credential.list, skills.list{personaId:None}, providers.list, models.list, models.catalog, models.efforts, search.all, schedule.list. Kept in order on purpose: ping (a client's barrier), mcp.auth_status (may refresh a token), backends.list and remote.devices (phone-specific shaping in the inline match). A spawned read starts only after earlier commands finish. Test: wire/tests.rs a_slow_read_answers_after_the_writes_behind_it_which_keep_their_order (Quiet gained a slow_read gate on capability_options; fails without the change). docs/wire.md lists the set. fmt, clippy -D warnings, full cargo test -p hotline-core green, contract.ts unchanged. Note for task-20: the worktree needs .speech-engine/ copied from the main checkout to build.
