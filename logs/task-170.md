---
id: task-170
title: "P2-55 (F): The dependency gates"
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:12.168Z"
relatedFiles:
  - deny.toml
  - Makefile
  - .github/workflows/ci.yml
updatedAt: "2026-10-18T04:43:43.216Z"
contract:
  status: in_progress
  validation:
    commands:
      - test -f deny.toml
      - grep -q 'deny' Makefile
      - grep -q 'machete' Makefile
      - grep -q 'bun audit' Makefile
      - make check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:45:57.426Z"
    reworkCount: 0
dependsOn:
  - task-169
  - task-124
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:31.127Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:45:13.239Z"
    ended: "2026-10-18T03:50:36.007Z"
    outcome: review
assignee: claude
completedAt: "2026-10-18T04:43:43.216Z"
---

## Description
Phase 2 group F (Gates), plan row 55 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The dependency gates.** Pinned machete, with the `tauri-build` false positive explained. cargo-deny advisories and `bun audit`, with unresolved upstream IDs (from 4, 5, and the unmaintained GTK and D-Bus crates) listed with a review condition

- Source: dead-code F4, tasks 15 and 17
- Files: `deny.toml`, `Makefile`, `.github/workflows/ci.yml`
- Collides with: 2–7
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T03:50:36.499Z: Not landed: 6 files are not committed in the worktree: send it back, or commit in your editor
