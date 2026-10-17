---
id: task-121
title: "P2-9 (B): Narrow `lib.rs`"
column: in-progress
position: 10
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:46.601Z"
relatedFiles:
  - crates/hotline-core/src/lib.rs
  - crates/hotline-core/src/
updatedAt: "2026-10-17T23:04:12.279Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T23:04:12.279Z"
    reworkCount: 0
dependsOn:
  - task-120
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T23:03:21.849Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T23:03:24.259Z"
assignee: claude
---

## Description
Phase 2 group B (Delete dead code), plan row 9 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Narrow `lib.rs`.** Make the nine modules no outside crate names `pub(crate)`, and delete whatever the compiler then reports as unused. Moved up from structure's step 19 (C10)

- Source: structure F12, task 19
- Files: `crates/hotline-core/src/lib.rs:14-48`, plus whatever the compiler reports
- Collides with: 16 (contract leaf) touches `thread`, `spending`, `linked`: land 9 first
- Size: S–M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-17T23:21:04.818Z: [claude] Made link_preview, linked, push, session, skills, spending, store, thread, websearch pub(crate) in lib.rs (checked first: no other crate names them; the workspace builds). Deleted what the compiler then reported: the re-exports session::{DeliverResult, Sent, MAX_LIVE_SIDES} and thread::{Seed, Surface}; QUIET_MS/SIDE_IDLE_MS are now re-exported only under cfg(test); the four unused provider ::new constructors (attempt() uses ::at); the never-read rank::Source.provider (the rank tests' src() helper lost its provider argument); Request::plain and Search::with_bounds moved into websearch/tests.rs because only tests call them; Indexer::forget and its test deleted. LEFT FOR LATER (George): nothing ever called Indexer::forget (the reference tree doesn't either), so delete_persona leaves a deleted teammate's rows in search.db, and search_all can still return them. If that matters, it's a behaviour fix with its own harness, and git has the old method. make check, make verify and make precheck (7 legs) pass. Commit 5f6885c8 on dispatch/task-121. Task 16 (contract leaf) can now proceed on thread/spending/linked.
