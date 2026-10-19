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
updatedAt: "2026-10-19T06:07:12.268Z"
---

## Description
store, log and sent, on hotline-contract and hotline-support. First cut the cycle: store uses thread::{Link, ThreadState}, plain types that move down to hotline-contract or into the store crate; thread itself (it reaches room and session) stays in hotline-core. The search index (store/search.rs) moves whole. Its tests move with it; the desk harness in crates/hotline-core/tests still passes unchanged.
