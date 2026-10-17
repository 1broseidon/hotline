---
id: task-104
title: "Audit: dead code and dependencies (Rust and window)"
column: todo
position: 2
priority: high
tags:
  - audit
  - dead-code
parentId: epic-12
createdAt: "2026-10-17T19:28:03.692Z"
---

## Description
Audit: dead code and dependencies.

- Unused pub items and modules, all 5 #[allow(dead_code)] (justify each or remove), cfg-gated code that never builds, unused features.
- Stale scripts/, packaging/, docs and fixtures.
- Unused dependencies (cargo machete; cargo +nightly udeps if available), duplicate crate versions (cargo tree -d), heavy deps with a lighter alternative, cargo deny advisories/licenses/bans.
- The same for ui/package.json.
- Report: docs/audits/0.40/dead-code-and-deps.md. List exact removals as proposed Phase 2 tasks, grouped so each lands alone.
