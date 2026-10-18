---
id: task-125
title: "P2-13 (B): Declare crate licenses"
column: todo
position: 14
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
updatedAt: "2026-10-18T00:14:43.052Z"
contract:
  status: ready
  validation:
    commands:
      - cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; sys.exit(0 if all(p['license'] for p in json.load(sys.stdin)['packages']) else 1)"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-123
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:42.917Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 2 group B (Delete dead code), plan row 13 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Declare crate licenses.** Add an SPDX expression in `[workspace.package]` and inherit it in all three crates

- Source: dead-code F4, task 7
- Files: `Cargo.toml`, three `Cargo.toml`s
- Collides with: 52 (lints table, same section)
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
