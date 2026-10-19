---
id: task-186
title: Start paired desks concurrently at launch
column: todo
position: 12
priority: high
tags:
  - startup
  - performance
parentId: epic-14
createdAt: "2026-10-18T23:40:56.313Z"
relatedFiles:
  - crates/hotline-app/src/desks.rs
updatedAt: "2026-10-19T00:34:01.867Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-app/src/desks.rs
      description: a desk that hangs does not delay the others; per-desk errors stay separate
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-app
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:33:56.531Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:34:01.867Z"
assignee: claude
---

## Description
Slice of task-21. hotline-app desks.rs start_all starts paired desks one by one, so one slow desk delays the rest. Start them concurrently (join_all with a per-desk timeout), keeping per-desk errors separate. Test that a desk that hangs doesn't delay the others. The other startup work stays in task-21.

Split from task-21, which keeps the rest and stays deferred.
