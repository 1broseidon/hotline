---
id: task-123
title: "P2-11 (B): Dependency surface"
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
updatedAt: "2026-10-18T01:54:00.448Z"
contract:
  status: delivered
  validation:
    commands:
      - "! git grep -n 'Win32_System_Memory' -- crates/hotline-core/Cargo.toml"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:46:40.913Z"
    reworkCount: 2
    deliveredAt: "2026-10-18T01:51:28.212Z"
    duration: 287
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
    pid: 39764
    at: "2026-10-18T01:42:48.694Z"
    ended: "2026-10-18T01:54:00.448Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T01:54:00.069Z"
---

## Description
Phase 2 group B (Delete dead code), plan row 11 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Dependency surface.** Rig without `derive` (explicitly `agent`, `reqwest`, `rustls`). Drop `Win32_System_Memory`

- Source: dead-code F8, tasks 11 and 12
- Files: `Cargo.toml:10`, `crates/hotline-core/Cargo.toml:114`, `Cargo.lock`
- Collides with: 2–5 (lockfile, core Cargo.toml)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T01:51:30.508Z: [claude] Done on dispatch/task-123 (7e2226cc). Root Cargo.toml: rig = 0.42.0 with default-features = false, features agent/reqwest/rustls (identical to rig's defaults minus derive; rig-core defaults are reqwest/derive/rustls, so derive drops at both levels). Lockfile loses rig-derive and convert_case 0.11.0 only. Core Cargo.toml: Win32_System_Memory removed from windows-sys; the Windows leg of precheck builds clean without it. make check and make precheck pass (7/7). Nothing left unwired; the lockfile still collides with rows 2–5 if they land after this.
- 2026-10-18T01:53:59.697Z: Landed on release/0.40.0
