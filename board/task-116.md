---
id: task-116
title: "P2-4 (A): Tauri updater ZIP path"
column: todo
position: 5
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:44.032Z"
relatedFiles:
  - crates/hotline-app/Cargo.toml
  - Cargo.lock
updatedAt: "2026-10-18T01:08:55.025Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-115
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:37.210Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59932
    at: "2026-10-18T01:08:55.025Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 4 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Tauri updater ZIP path.** Upgrade the owning plugin to a release with patched extraction, or file an upstream blocker with the IDs. Do not force ZIP 9 under a ZIP 4 API

- Source: dead-code F1, task 3
- Files: `crates/hotline-app/Cargo.toml`, `Cargo.lock`
- Collides with: 2, 3, 5 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
