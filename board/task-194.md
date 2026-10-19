---
id: task-194
title: "hotline-contract: the wire and tape types as their own crate"
column: todo
position: 2
priority: high
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/contract.rs
  - crates/hotline-core/src/contract
  - ui/src/generated/contract.ts
parentId: epic-16
createdAt: "2026-10-19T06:06:58.782Z"
---

## Description
Move contract.rs and contract/ into crates/hotline-contract. It depends on no other hotline crate (replace the doc link to crate::paths::thread_key with words). hotline-core depends on it and keeps `pub use hotline_contract as contract` (or moves callers; your choice, say which in the commit). The ts-rs export moves with the types and still writes ui/src/generated/contract.ts byte-identical; check the generation test and make check's contract sync still run it, and that hotline-mobile's sync:contract source path (../hotline/ui/src/generated) is unchanged. hotline-app and hotline-cli build.
