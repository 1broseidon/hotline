---
id: task-147
title: "P2-38 (D): The remote harness connects over loopback"
column: todo
position: 36
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:00.111Z"
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/network.rs
  - crates/hotline-core/src/remote.rs
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-cli/tests/serve.rs
  - crates/hotline-cli/tests/relay.rs
  - crates/hotline-cli/tests/pair.rs
updatedAt: "2026-10-18T05:21:26.528Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-146
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:07.611Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:21:26.528Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 38 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The remote harness connects over loopback.** The interface test keeps real addresses. Then let `Remote` and `ServeOptions` take a bound listener in tests, ending the free-port race

- Source: tests-and-docs T1, T11, task 3
- Files: `remote/tests.rs:62-163`, `remote/network.rs`, `crates/hotline-cli/tests/{serve,relay,pair}.rs`
- Collides with: 25, 37
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
