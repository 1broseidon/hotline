---
id: task-208
title: "hotline-room: RoomHandle and the room's plain pieces as their own crate"
column: todo
position: 9
priority: high
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/wire/room_handle.rs
  - crates/hotline-core/src/room.rs
  - crates/hotline-core/src/linked.rs
  - crates/hotline-core/src/session/ledger.rs
  - crates/hotline-core/src/voice/ledger.rs
  - crates/hotline-core/src/voice/metering.rs
  - crates/hotline-core/src/voice/spoken.rs
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-core/src/lib.rs
  - docs/standards.md
  - docs/security.md
  - AGENTS.md
  - docs/adr/0007-split-hotline-core.md
parentId: epic-18
createdAt: "2026-10-19T14:07:41.570Z"
dependsOn:
  - task-211
updatedAt: "2026-10-19T14:09:46.980Z"
---

## Description
ADR 0007 step 1, second half. task-211 has already cut RoomHandle's five core-typed signatures (voice(), remote(), links() left the trait; AuthStatus and import::Report are plain data): read its notes and commit first. Now make crates/hotline-room and move down, with RoomHandle (wire/room_handle.rs): room (with mcp::normalize_servers beside it), linked, session::ledger, session::ProviderAuth, wire::same_secret, driver::clip and failure::redact_text, desk::install_crypto_provider, computer::HOME_MOUNT, AuthStatus and import::Report, and voice's ledger, metering and spoken (contract, log and each other only). The three test rooms (CoreHandle, Quiet, Computered) keep implementing RoomHandle. Each moved module renames its own crate:: paths to the new crates; the core keeps `pub use` re-exports only while callers remain. The crate gets the workspace lints, a license and a crate doc saying what it owns and what it must never depend on (hotline-core, Tauri). docs/standards.md's module map, docs/security.md citations and AGENTS.md follow the moves. Name the crate in the ADR if hotline-room reads wrong (the validation looks for the moved files' absence from the core, not the name). In ADR 0007, list each moved file's old path and new path: epic-17's task-202 names room.rs and task-207 names voice/spoken.rs, and that list is how they find them.
