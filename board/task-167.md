---
id: task-167
title: "P2-52 (F): The `[workspace.lints]` table in one change"
column: in-progress
position: 56
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:10.558Z"
relatedFiles:
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-app/Cargo.toml
  - crates/hotline-cli/Cargo.toml
updatedAt: "2026-10-18T10:49:39.580Z"
contract:
  status: in_progress
  validation:
    commands:
      - grep -q '^\[workspace.lints' Cargo.toml
      - grep -q 'rust-version' Cargo.toml
      - for c in core app cli; do grep -q '^\[lints\]' crates/hotline-$c/Cargo.toml || exit 1; done
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T10:49:39.580Z"
    reworkCount: 0
dependsOn:
  - task-160
  - task-156
  - task-175
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:27.779Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T10:49:32.710Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 52 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The `[workspace.lints]` table in one change.** It holds every lint that 39–42 made clean, the zero-hit denies, and `allow_attributes_without_reason`. Add `rust-version`, and `[lints] workspace = true` in all three crates. `-D warnings` stays in the Makefile

- Source: lints, tasks 1 and 4 (C4, C5)
- Files: `Cargo.toml`, three crate manifests
- Collides with: 13
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
