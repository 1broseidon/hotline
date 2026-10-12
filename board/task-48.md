---
id: task-48
title: "Core: isolate stream writes and move search indexing off append paths"
column: todo
position: 35
description: Split from task-19, preserving BRO-240. Log still owns one global writer mutex (log/mod.rs:105); append opens a file for each write and compaction holds the same lock. Preserve one writer per stream while letting unrelated streams advance. Move full-load search indexing and commits off the append path with durable catch-up; reduce the settings policy lock only without weakening mutation/revocation ordering. Prove concurrent append/compaction correctness, no lost index updates across restart, and immediate authority revocation. Coordinate with task-19 and task-20.
priority: medium
tags:
  - performance
  - core
relatedFiles:
  - crates/hotline-core/src/log/mod.rs
  - crates/hotline-core/src/store/search.rs
parentId: epic-5
createdAt: "2026-10-12T04:20:51.009Z"
---

## Description
Split from task-19, preserving BRO-240. Log still owns one global writer mutex (log/mod.rs:105); append opens a file for each write and compaction holds the same lock. Preserve one writer per stream while letting unrelated streams advance. Move full-load search indexing and commits off the append path with durable catch-up; reduce the settings policy lock only without weakening mutation/revocation ordering. Prove concurrent append/compaction correctness, no lost index updates across restart, and immediate authority revocation. Coordinate with task-19 and task-20.
