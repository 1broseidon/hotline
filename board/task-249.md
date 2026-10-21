---
id: task-249
title: cargo-nextest runs the tests in CI
column: backlog
position: 49
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:28.148Z"
dependsOn:
  - task-246
updatedAt: "2026-10-21T05:03:57.365Z"
---

## Description
Done when:
- CI runs the workspace tests with cargo-nextest (installed at a pinned version, like the other tools), and doc tests still run with cargo test --doc. make check uses nextest when it's installed and falls back to cargo test.
- The same tests run before and after (counts in the PR), with no retries hiding failures.
- The PR records before/after test-step timings on each OS.
