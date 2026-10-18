---
id: task-123
title: "P2-11 (B): Dependency surface"
column: todo
position: 12
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:47.628Z"
relatedFiles:
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - Cargo.lock
updatedAt: "2026-10-18T01:38:37.725Z"
contract:
  status: ready
  validation:
    commands:
      - "! git grep -n 'Win32_System_Memory' -- crates/hotline-core/Cargo.toml"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-117
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:40.587Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59932
    at: "2026-10-18T01:38:37.725Z"
assignee: claude
---

## Description
Phase 2 group B (Delete dead code), plan row 11 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Dependency surface.** Rig without `derive` (explicitly `agent`, `reqwest`, `rustls`). Drop `Win32_System_Memory`

- Source: dead-code F8, tasks 11 and 12
- Files: `Cargo.toml:10`, `crates/hotline-core/Cargo.toml:114`, `Cargo.lock`
- Collides with: 2–5 (lockfile, core Cargo.toml)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
