---
id: task-144
title: "P2-35 (D): Discovery client is fallible"
column: todo
position: 33
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:58.554Z"
relatedFiles:
  - crates/hotline-core/src/providers/discovery.rs
updatedAt: "2026-10-17T22:46:58.194Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 35 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Discovery client is fallible.** Remove `Default`, propagate the error, and keep the previous list

- Source: errors F2, task 2
- Files: `providers/discovery.rs:95,219,249,293`
- Collides with: 39 (`future_not_send` at `:182`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
