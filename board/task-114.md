---
id: task-114
title: "P2-2 (A): Patch rustls"
column: todo
position: 3
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:43.005Z"
---

## Description
Phase 2 group A (Security), plan row 2 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch rustls** from 0.23.45 to `>=0.23.46`, that crate only. cargo-deny no longer reports RUSTSEC-2026-0346 to 0349

- Source: dead-code F2, task 1
- Files: `Cargo.lock`
- Collides with: 3, 4, 5 (lockfile): land one at a time
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
