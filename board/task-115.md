---
id: task-115
title: "P2-3 (A): Patch ACP ZIP extraction (F1)"
column: in-progress
position: 4
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:43.518Z"
relatedFiles:
  - crates/hotline-core/Cargo.toml
  - Cargo.lock
  - crates/hotline-core/src/driver/acp/install.rs
  - docs/security.md
updatedAt: "2026-10-18T00:32:59.528Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:32:59.528Z"
    reworkCount: 0
dependsOn:
  - task-113
  - task-114
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:36.022Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:32:05.196Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 3 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch ACP ZIP extraction (F1).** This is a capability change; see below

- Source: dead-code F1, task 2; also lints `case_sensitive_file_extension_comparisons` at `install.rs:112` and dead-code F8 (`zip` features)
- Files: `crates/hotline-core/Cargo.toml:42`, `Cargo.lock`, `crates/hotline-core/src/driver/acp/install.rs` and its tests, `docs/security.md` (new row)
- Collides with: 2, 4, 5 (lockfile); 11 (Cargo.toml)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
