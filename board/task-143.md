---
id: task-143
title: "P2-34 (D): One boot that returns errors"
column: todo
position: 32
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:58.025Z"
relatedFiles:
  - crates/hotline-app/src/lib.rs
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-core/src/door.rs
  - crates/hotline-core/src/desk.rs
updatedAt: "2026-10-18T02:23:40.711Z"
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
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:03.139Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:23:40.711Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 34 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One boot that returns errors.** A core function that both `hotline serve` and the app call: room lock, backend, Desk, Remote, Door, and one token minter that never falls back to weak entropy. The desktop's four startup `expect`s and the CLI's runtime, signal and entropy `expect`s become errors at one outer handler, with an unsuccessful exit. The relay-only, file-store and drain differences stay as arguments (C13)

- Source: structure F13, task 20; errors F1, F3, tasks 1 and 3
- Files: `crates/hotline-app/src/lib.rs:66,307-362,570`, `crates/hotline-cli/src/serve.rs:161-282`, `door.rs:173`, `desk.rs`
- Collides with: none after 14
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
