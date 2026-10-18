---
id: task-125
title: "P2-13 (B): Declare crate licenses"
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:48.659Z"
relatedFiles:
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-app/Cargo.toml
  - crates/hotline-cli/Cargo.toml
updatedAt: "2026-10-18T02:06:56.403Z"
contract:
  status: delivered
  validation:
    commands:
      - cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; sys.exit(0 if all(p['license'] for p in json.load(sys.stdin)['packages']) else 1)"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:55:13.979Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T02:04:23.966Z"
    duration: 550
dependsOn:
  - task-123
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:42.917Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T01:54:01.588Z"
    ended: "2026-10-18T02:06:56.403Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T02:06:56.022Z"
---

## Description
Phase 2 group B (Delete dead code), plan row 13 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Declare crate licenses.** Add an SPDX expression in `[workspace.package]` and inherit it in all three crates

- Source: dead-code F4, task 7
- Files: `Cargo.toml`, three `Cargo.toml`s
- Collides with: 52 (lints table, same section)
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T02:04:23.649Z: [claude] Done on dispatch/task-125 (448c3f90): `license = "MIT OR Apache-2.0"` in [workspace.package] (matches LICENSE-MIT/LICENSE-APACHE and the README), `license.workspace = true` in core, app and cli right after edition. cargo metadata shows all three licensed; lockfile untouched; make check and make precheck (7/7) pass. For row 52: the lints/rust-version lines go in the same [workspace.package] section and each crate's [package], so expect a trivial adjacent-line conflict. For row 56: crates are no longer unlicensed, so cargo-deny's allow list only needs to cover registry crates. Nothing left unwired.
- 2026-10-18T02:06:55.654Z: Landed on release/0.40.0
