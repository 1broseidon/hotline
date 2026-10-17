---
id: task-150
title: "P2-41 (D): Argument groups"
column: todo
position: 39
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:01.678Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 41 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Argument groups.** Nine `too_many_arguments` refactors, one per function chain, with `make verify` for wire, voice and remote

- Source: lints, task 5
- Files: `wire/subscribe.rs` (was `wire/mod.rs:1903,2270`), `voice/{call,synthesis}.rs`, `wire/commands/persona.rs`, `session/exchanges.rs:176`, `remote.rs:737`, `driver/rig/turn.rs:689`
- Collides with: 28–31 (land first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
