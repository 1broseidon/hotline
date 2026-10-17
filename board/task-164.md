---
id: task-164
title: "P2-D-4 (E): Present-tense wording and what `make check` runs"
column: todo
position: 53
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:08.998Z"
relatedFiles:
  - crates/hotline-core/src/lib.rs
  - Makefile
  - docs/design.md
  - docs/development.md
  - AGENTS.md
updatedAt: "2026-10-17T22:47:21.105Z"
---

## Description
Phase 2 group E (Docs), plan row D-4 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Present-tense wording and what `make check` runs.** Fix `lib.rs:5`, `Makefile:42` and `design.md:663` (history stays labelled as history). Remove the test counts from development.md and list what `make check` really runs there and in `AGENTS.md:55`. Document `HOTLINE_EXTRA_DESKS` and `HOTLINE_DEV_SIGN_IDENTITY`

- Source: dead-code F10, task 14; tests-and-docs D6, D11, tasks 14 and 17 (env part)
- Files: `crates/hotline-core/src/lib.rs:5`, `Makefile:42`, `docs/design.md:663`, `docs/development.md:108-116,499`, `AGENTS.md:55`
- Collides with: 9 (`lib.rs`)
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
