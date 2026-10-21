---
id: task-249
title: cargo-nextest runs the tests in CI
column: in-progress
position: 0
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:28.148Z"
dependsOn:
  - task-246
updatedAt: "2026-10-21T05:04:17.307Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T05:03:58.286Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T05:04:00.303Z"
assignee: codex
---

## Description
Done when:
- CI runs the workspace tests with cargo-nextest (installed at a pinned version, like the other tools), and doc tests still run with cargo test --doc. make check uses nextest when it's installed and falls back to cargo test.
- The same tests run before and after (counts in the PR), with no retries hiding failures.
- The PR records before/after test-step timings on each OS.

## Log
- 2026-10-21T05:05:15.665Z: [codex] In progress on dispatch/task-249: preserving task-246 integration targets; nextest must delegate hotline-contract to one libtest process (ts-rs exports) and shell_path to its existing harness-free entry point. Baseline local run compiling; per-OS after CI evidence requires reviewer push, forbidden in this dispatch.
