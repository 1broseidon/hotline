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
---

## Description
One crate for the leaves that hold no Hotline state: credentials (atomic_write and friends), process_windows, images, thumbs, fence, room_lock, log_file, and paths once its one link to linked (is_linked, a string test) moves into paths itself or into hotline-contract. Pick the name in the ADR if hotline-support reads wrong; keep it one crate, not seven. room_lock is the one-writer-per-data-dir guard (desk.lock): its tests move with it and still prove a second desk is refused. Unsafe code keeps its SAFETY comments.
