---
id: task-136
title: "P2-27 (C): `driver/acp.rs` code"
column: todo
position: 25
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.356Z"
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - crates/hotline-core/src/driver/acp/oauth_proxy.rs
  - crates/hotline-core/src/driver/acp/live.rs
  - crates/hotline-core/src/driver/acp/translate.rs
  - crates/hotline-core/src/driver/acp/session.rs
updatedAt: "2026-10-17T22:46:56.306Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 27 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`driver/acp.rs` code** → `oauth_proxy.rs`, `live.rs`, `translate.rs`, `session.rs`

- Source: structure F2, task 13
- Files: `driver/acp.rs`
- Collides with: 3 (`acp/install.rs` is untouched, but land 3 first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
