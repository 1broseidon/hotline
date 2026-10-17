---
id: task-170
title: "P2-55 (F): The dependency gates"
column: todo
position: 59
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
updatedAt: "2026-10-17T22:47:22.710Z"
---

## Description
Phase 2 group F (Gates), plan row 55 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The dependency gates.** Pinned machete, with the `tauri-build` false positive explained. cargo-deny advisories and `bun audit`, with unresolved upstream IDs (from 4, 5, and the unmaintained GTK and D-Bus crates) listed with a review condition

- Source: dead-code F4, tasks 15 and 17
- Files: `deny.toml`, `Makefile`, `.github/workflows/ci.yml`
- Collides with: 2–7
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
