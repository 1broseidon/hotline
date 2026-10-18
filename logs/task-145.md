---
id: task-145
title: "P2-36 (D): Lock recovery"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:59.075Z"
relatedFiles:
  - crates/hotline-core/src/remote.rs
  - crates/hotline-core/src/remote/admission.rs
  - crates/hotline-core/src/remote/relay.rs
  - crates/hotline-core/src/remote/bridge.rs
  - crates/hotline-app/src/transfer.rs
updatedAt: "2026-10-18T05:06:39.548Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:39:03.255Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:05.391Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:38:57.284Z"
    ended: "2026-10-18T05:06:39.548Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T05:06:38.425Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 36 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Lock recovery.** Use the one `PoisonError::into_inner` idiom for `Permit::drop` and admission first, then relay, then Remote and bridge, then the five `transfer.rs` sites. One domain per commit, with partial mutations reviewed

- Source: errors F4, tasks 4 and 5
- Files: `remote/{admission,relay,bridge}.rs`, `remote.rs`, `crates/hotline-app/src/transfer.rs`
- Collides with: 39, 51 (`significant_drop_in_scrutinee` lens)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:06:37.408Z: Landed on release/0.40.0
