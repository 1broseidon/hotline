---
id: task-197
title: "hotline-store: the tape, the log and the search index as their own crate"
column: todo
position: 5
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/store.rs
  - crates/hotline-core/src/store
  - crates/hotline-core/src/log.rs
  - crates/hotline-core/src/log
  - crates/hotline-core/src/sent.rs
  - crates/hotline-core/src/thread.rs
parentId: epic-16
createdAt: "2026-10-19T06:07:00.407Z"
dependsOn:
  - task-194
  - task-196
updatedAt: "2026-10-19T06:10:20.929Z"
---

## Description
store, log and sent, on hotline-contract and hotline-support. First cut the cycle: store/search.rs uses thread::{Link, ThreadState} (Link is in thread/link.rs, ThreadState in thread.rs), plain types that move down to hotline-contract or into the store crate; thread itself (it reaches room and session) stays in hotline-core. sent reaches images and thumbs (in hotline-support by now), log reaches credentials (atomic_write) and paths. The search index (store/search.rs) moves whole. Its tests move with it; the desk harness in crates/hotline-core/tests still passes unchanged (validation refuses any diff to tests/desk.rs or tests/desk/). Tapes stay byte-compatible: no event shape or segment layout change. If Link or ThreadState carries a ts-rs derive, contract.ts must still come out byte-identical.

Lead's note: this runs after task-196 and before task-198 because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md); run in parallel they would not land cleanly.
