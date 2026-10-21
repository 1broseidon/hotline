---
id: task-246
title: One integration-test binary per crate
column: in-progress
position: 1
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:26.705Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T02:18:39.157Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T02:18:40.437Z"
assignee: codex
updatedAt: "2026-10-21T02:18:58.332Z"
---

## Description
Done when:
- hotline-core, hotline-cli and hotline-app each build one integration-test binary (autotests = false and one [[test]] root whose modules are today's tests/*.rs files, left where they are, so concurrent work on those files doesn't conflict). A test that needs its own process (process-global state, environment, exit) stays a separate [[test]], with the reason written beside it.
- cargo test --workspace passes on Linux, macOS and Windows in CI, with the same tests run before and after (the counts are equal and in the PR).
- The PR records before/after for local cargo test --workspace --no-run (cold, and after touching hotline-core) and for CI's test step on each OS.

## Log
- 2026-10-21T02:20:16.109Z: [codex] Baseline cold build running at fd41e003. Isolation audit: core tls requires an unset rustls global; core processes and voice mutate PATH through common::browser; app shell_path runs a harness-free child with a private environment. These remain explicit separate targets. Consolidated roots will use #[path] to preserve existing file/module layout (task-specific exception to standards).
