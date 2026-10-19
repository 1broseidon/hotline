---
id: task-195
title: "hotline-websearch: web search as its own crate"
column: todo
position: 3
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/websearch.rs
  - crates/hotline-core/src/websearch
parentId: epic-16
createdAt: "2026-10-19T06:06:59.331Z"
dependsOn:
  - task-194
  - task-198
updatedAt: "2026-10-19T06:10:21.526Z"
---

## Description
websearch (about 6,200 lines) reaches only contract. Move it to crates/hotline-websearch depending on hotline-contract; its HTTP and provider dependencies move with it and leave hotline-core's Cargo.toml if nothing else uses them (cargo machete agrees, and make check runs it).

Lead's note: this runs last of the moves, after task-198, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md); it is the smallest and lowest-risk, so it waits rather than the store and tools moves.
