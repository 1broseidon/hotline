---
id: task-114
title: "P2-2 (A): Patch rustls"
column: review
position: 3
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:43.005Z"
relatedFiles:
  - Cargo.lock
updatedAt: "2026-10-17T22:59:04.327Z"
contract:
  status: delivered
  validation:
    commands:
      - "! grep -A1 '^name = \"rustls\"$' Cargo.lock | grep -q 'version = \"0.23.45\"'"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T22:49:32.108Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T22:59:04.327Z"
    duration: 572
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-17T22:48:58.989Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T22:49:03.304Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 2 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch rustls** from 0.23.45 to `>=0.23.46`, that crate only. cargo-deny no longer reports RUSTSEC-2026-0346 to 0349

- Source: dead-code F2, task 1
- Files: `Cargo.lock`
- Collides with: 3, 4, 5 (lockfile): land one at a time
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-17T22:59:05.949Z: [claude] rustls 0.23.45 -> 0.23.46 via `cargo update -p rustls --precise 0.23.46`; Cargo.lock only (rustls's getrandom moved 0.4.3 -> 0.3.4 with it). cargo-deny no longer reports RUSTSEC-2026-0346..0349; it still fails on the ZIP advisories (tasks 3-5) and unmaintained crates (unic-*, zbus/zvariant, proc-macro-error), which are not this task's. make precheck and make check pass. Committed on dispatch/task-114. Next lockfile tasks (3, 4, 5) should land after this one and may need a lockfile merge.
