---
id: task-130
title: "P2-18–22 (C): `session/mod.rs` in five parts:"
column: todo
position: 19
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:51.248Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 18–22 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`session/mod.rs` in five parts:** preamble and asking; secrets; computer; prompt and driver_info; start and chapters

- Source: structure F1, tasks 5–9
- Files: `session.rs`, `session/{preamble,asking,secrets,computer,prompt,driver_info,start,chapters}.rs`
- Collides with: 23; 40 (expects in `session/mod.rs:813,2630`)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
