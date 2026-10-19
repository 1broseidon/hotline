---
id: task-196
title: "hotline-support: the small leaf utilities as one crate"
column: todo
position: 4
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/credentials.rs
  - crates/hotline-core/src/process_windows.rs
  - crates/hotline-core/src/images.rs
  - crates/hotline-core/src/thumbs.rs
  - crates/hotline-core/src/fence.rs
  - crates/hotline-core/src/room_lock.rs
  - crates/hotline-core/src/log_file.rs
  - crates/hotline-core/src/paths.rs
parentId: epic-16
createdAt: "2026-10-19T06:06:59.870Z"
dependsOn:
  - task-194
updatedAt: "2026-10-19T06:10:31.816Z"
---

## Description
One crate for the leaves that hold no Hotline state: credentials (atomic_write and friends, credentials/), process_windows, images, thumbs (reaches images), fence, room_lock, log_file (reaches paths; its harness=false integration test crates/hotline-core/tests/log_file.rs and the [[test]] entry move with it), and paths once its one link to linked (is_linked, a string test) moves into paths itself or into hotline-contract. Use the name ADR 0007 settled (hotline-support unless it says otherwise); keep it one crate, not seven. room_lock is the one-writer-per-data-dir guard (desk.lock): its tests move with it and still prove a second desk is refused. Unsafe code keeps its SAFETY comments.

Lead's note: this runs before task-197 and task-198 (both need it) and alone, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md).
