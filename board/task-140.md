---
id: task-140
title: "P2-31 (C): The six other over-budget files"
column: review
position: 2
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:56.439Z"
relatedFiles:
  - crates/hotline-core/src/session/peers.rs
  - crates/hotline-core/src/tools/workspace.rs
  - crates/hotline-core/src/session/sides.rs
  - crates/hotline-core/src/computer.rs
  - crates/hotline-core/src/driver/rig.rs
  - crates/hotline-core/src/driver/rig/turn.rs
updatedAt: "2026-10-18T05:51:59.780Z"
contract:
  status: in_progress
  validation:
    commands:
      - for f in session/peers.rs tools/workspace.rs session/sides.rs computer.rs driver/rig.rs driver/rig/turn.rs; do test $(wc -l < crates/hotline-core/src/$f) -le 1500 || exit 1; done
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:23:59.384Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T05:51:51.253Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:51:59.780Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 31 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The six other over-budget files**, one commit each: `session/peers.rs`, `tools/workspace.rs`, `session/sides.rs`, `computer/mod.rs`, `driver/rig.rs`, `driver/rig/turn.rs` (its tests, plus the `run_inner` phases with the retry rule kept in one place)

- Source: structure F8, task 17
- Files: those files
- Collides with: 41 (`rig/turn.rs:689`), 42 (rig `Failure` sites), 47 (`turn.rs` sleeps)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:29:15.423Z: Not landed: Another landing in this repository is still going (pid 35147 on Georges-Mac-Studio, since 2026-10-18T05:27:51.048Z): nothing was merged, try again once it is done
- 2026-10-18T05:51:51.098Z: Landing sent back (1 of 1): dispatch/task-140 conflicts with release/0.40.0 in docs/security.md. On release/0.40.0 since this branch started: 4d8d2faa security.md and the front ADR cite each moved test at the file it now lives in (task-141); bf4110d1 The ACP driver's code is filed by concern beside it: the OAuth proxy, the live connection, the translation and the session's start, so acp.rs holds the driver and the child (task-136); ed08b065 wire.rs keeps the shared helpers and each concern of the socket lives in its own file under wire/, so no production file passes the 1,500-line budget (task-138); 27d4fbb9 The contract is one module filed by concern in nine files, each re-exported, so every crate::contract path and contract.ts hold unchanged (task-129). Merge release/0.40.0 into this branch (git merge release/0.40.0), keep what both sides meant, commit the merge, and run the checks: for f in session/peers.rs tools/workspace.rs session/sides.rs computer.rs driver/rig.rs driver/rig/turn.rs; do test $(wc -l < crates/hotline-core/src/$f) -le 1500 || exit 1; done && git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts && make precheck && make check.
