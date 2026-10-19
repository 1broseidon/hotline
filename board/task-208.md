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
parentId: epic-18
createdAt: "2026-10-19T14:07:41.570Z"
---

## Description
ADR 0007 step 1. First cut RoomHandle's five core-typed signatures: voice(), remote() and links() leave the trait (the wire is handed the calls, the remote and the links when it is built, as the desk already holds them), and driver::auth::AuthStatus and import::Report move down as plain data. Then move down, with RoomHandle: room (with mcp::normalize_servers beside it), linked, session::ledger, session::ProviderAuth, wire::same_secret, driver::clip and failure::redact_text, desk::install_crypto_provider, computer::HOME_MOUNT, and voice's ledger, metering and spoken (contract, log and each other only). The three test rooms (CoreHandle, Quiet, Computered) keep implementing RoomHandle. Name the crate in the ADR if hotline-room reads wrong.
