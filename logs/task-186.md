---
id: task-186
title: Start paired desks concurrently at launch
priority: high
tags:
  - startup
  - performance
parentId: epic-14
createdAt: "2026-10-18T23:40:56.313Z"
relatedFiles:
  - crates/hotline-app/src/desks.rs
updatedAt: "2026-10-19T00:47:01.944Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T00:34:14.269Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:41:17.206Z"
    duration: 423
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
    ended: "2026-10-19T00:41:25.877Z"
    outcome: review
assignee: claude
completedAt: "2026-10-19T00:47:01.944Z"
---

## Description
Slice of task-21. hotline-app desks.rs start_all starts paired desks one by one, so one slow desk delays the rest. Start them concurrently (join_all with a per-desk timeout), keeping per-desk errors separate. Test that a desk that hangs doesn't delay the others. The other startup work stays in task-21.

Split from task-21, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:41:21.419Z: [claude] Done on dispatch/task-186 (fe1c50f3). Host::start_all now calls start_each, which runs every desk's Bridge::start together with join_all, each under START_TIMEOUT (5s). A desk that times out goes through the same settle path as one that fails: it is listed unreachable with "The local desk bridge did not start in time." and retried. The test a_desk_that_hangs_at_launch_does_not_delay_the_others runs on a paused tokio clock (test-util is a dev-dependency) so the failing desk's real-time retry cannot land before the asserts; it fails if the desks are started one by one. For the rest of task-21: start_each takes an opener closure so tests can inject hangs; pairing again still goes through start(), which has no timeout. To build in a fresh worktree, copy .speech-engine/ over or run make speech-engine.
- 2026-10-19T00:47:01.540Z: Landed on release/0.40.0
