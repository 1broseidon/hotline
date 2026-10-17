---
id: task-115
title: "P2-3 (A): Patch ACP ZIP extraction (F1)"
column: todo
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
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-113
  - task-114
---

## Description
Phase 2 group A (Security), plan row 3 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch ACP ZIP extraction (F1).** This is a capability change; see below

- Source: dead-code F1, task 2; also lints `case_sensitive_file_extension_comparisons` at `install.rs:112` and dead-code F8 (`zip` features)
- Files: `crates/hotline-core/Cargo.toml:42`, `Cargo.lock`, `crates/hotline-core/src/driver/acp/install.rs` and its tests, `docs/security.md` (new row)
- Collides with: 2, 4, 5 (lockfile); 11 (Cargo.toml)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
