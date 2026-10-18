---
id: task-187
title: Run read-only global commands without blocking the socket
column: todo
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
updatedAt: "2026-10-18T23:54:20.975Z"
contract:
  status: ready
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
assignee: claude
---

## Description
Slice of task-20. wire/socket.rs: per-teammate lanes exist (:231), but other commands are awaited inline (:244-269). Spawn the non-mutating global reads (models.list, search, voice.status and similar), as file reads already are (:445-464). Keep every mutating command, settings.update included, in order. Test that a slow read doesn't hold up the next command.

Split from task-20, which keeps the rest and stays deferred.
