---
id: task-171
title: "P2-56 (F): The license gate"
column: in-progress
position: 60
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:12.690Z"
relatedFiles:
  - deny.toml
  - Makefile
  - THIRD-PARTY-NOTICES.md
updatedAt: "2026-10-18T10:14:28.874Z"
contract:
  status: in_progress
  validation:
    commands:
      - grep -q '^\[licenses\]' deny.toml
      - make check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T05:12:20.604Z"
    reworkCount: 0
dependsOn:
  - task-170
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T10:14:25.574Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T10:14:28.874Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 56 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The license gate.** A reviewed cargo-deny allow list, and a UI license and notice inventory covering fonts, bundled distributions and the native speech assets

- Source: dead-code F4, task 18
- Files: `deny.toml`, the notice inventory
- Collides with: 13, 55
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
