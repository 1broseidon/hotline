---
id: task-148
title: "P2-39 (D): Lint autofix slice"
column: todo
position: 37
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:00.635Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 39 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Lint autofix slice.** The fifteen "[1]" lints from lints.md, one commit per lint, followed by an unfiltered `cargo test -p hotline-core`. Prove each slice clean with a one-off `cargo clippy -- -W clippy::<lint>`; the table comes later (C4)

- Source: lints, task 2
- Files: about 700 sites workspace-wide
- Collides with: every split in C: land after 33
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
