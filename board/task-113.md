---
id: task-113
title: "P2-1 (A): security.md proves what it cites"
column: todo
position: 2
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:42.496Z"
---

## Description
Phase 2 group A (Security), plan row 1 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**security.md proves what it cites.** Re-point and re-word the four rows (verdicts below), and write the missing proof for `:1426`. George decides `:683`

- Source: tests-and-docs T3, task 1
- Files: `docs/security.md:330`, `:683`, `:703`, `:1426`; a new test in `session/tests.rs`
- Collides with: 23 (`session/tests.rs` split): land first
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
