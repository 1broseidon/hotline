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
updatedAt: "2026-10-21T02:23:11.800Z"
decisions:
  - decision-7
---

## Description
Done when:
- hotline-core, hotline-cli and hotline-app each build one integration-test binary (autotests = false and one [[test]] root whose modules are today's tests/*.rs files, left where they are, so concurrent work on those files doesn't conflict). A test that needs its own process (process-global state, environment, exit) stays a separate [[test]], with the reason written beside it.
- cargo test --workspace passes on Linux, macOS and Windows in CI, with the same tests run before and after (the counts are equal and in the PR).
- The PR records before/after for local cargo test --workspace --no-run (cold, and after touching hotline-core) and for CI's test step on each OS.

## Log
- 2026-10-21T02:20:16.109Z: [codex] Baseline cold build running at fd41e003. Isolation audit: core tls requires an unset rustls global; core processes and voice mutate PATH through common::browser; app shell_path runs a harness-free child with a private environment. These remain explicit separate targets. Consolidated roots will use #[path] to preserve existing file/module layout (task-specific exception to standards).
- 2026-10-21T02:30:32.118Z: [codex] Committed consolidation and isolation correction (7a58ce20, 00a3fd2d). Final layout is core 5, CLI 1, app 2 binaries (20 -> 8): acp_shutdown also needs isolation because end_every_agent kills other tests' children. Inventory matches all 2,180 test function names/multiplicities; baseline execution 2,164 passed / 16 ignored. Final measurements and make check/precheck underway; CI after-results cannot exist until reviewer pushes the branch.
- 2026-10-21T02:33:39.630Z: [codex] Ready for review on dispatch/task-246 at 40c5a982; worktree clean, no push. Consolidated 20 integration binaries to 8 with five documented isolation exceptions and a CI registration guard. make check and all 8 precheck legs pass; before/after workspace counts both 2,164 passed + 16 ignored, full 2,180-name inventories equal. Final cold no-run 154.96 -> 129.40 s; core-touch rebuild 13.89 -> 9.21 s. Report/PR handoff: docs/audits/build/task-246.md. Next: reviewer pushes branch, obtains Linux/macOS/Windows after CI counts/timings and remaining baseline Windows result, and includes those in PR before merge; this evidence is pending under the no-push instruction.
