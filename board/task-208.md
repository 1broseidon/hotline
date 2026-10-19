---
id: task-208
title: "hotline-room: RoomHandle and the room's plain pieces as their own crate"
column: in-progress
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
updatedAt: "2026-10-19T14:41:18.541Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-room/Cargo.toml
      description: the room crate (or the name the ADR gives it)
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: step 1 done, with each moved file's old and new path
  validation:
    commands:
      - "! test -e crates/hotline-core/src/room.rs"
      - "! test -e crates/hotline-core/src/linked.rs"
      - "! test -e crates/hotline-core/src/session/ledger.rs"
      - "! test -e crates/hotline-core/src/wire/room_handle.rs"
      - "! test -e crates/hotline-core/src/voice/ledger.rs"
      - "! test -e crates/hotline-core/src/voice/metering.rs"
      - "! test -e crates/hotline-core/src/voice/spoken.rs"
      - "! grep -rq 'pub trait RoomHandle' crates/hotline-core/src"
      - grep -rlq 'pub trait RoomHandle' crates/*/src
      - "! grep -lqE '^hotline-core[[:space:]]*=' $(grep -rl 'pub trait RoomHandle' crates/*/src | sed 's#/src/.*#/Cargo.toml#' | sort -u)"
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - Moves only, no behaviour change, no single-implementation trait (docs/standards.md)
    - ui/src/generated/contract.ts byte-identical after an unfiltered cargo test
    - The new crate never depends on hotline-core or Tauri
  metrics:
    readyAt: "2026-10-19T14:10:20.361Z"
    pickedUpAt: "2026-10-19T14:41:18.541Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T14:41:00.706Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T14:41:02.657Z"
---

## Description
ADR 0007 step 1, second half. task-211 has already cut RoomHandle's five core-typed signatures (voice(), remote(), links() left the trait; AuthStatus and import::Report are plain data): read its notes and commit first. Now make crates/hotline-room and move down, with RoomHandle (wire/room_handle.rs): room (with mcp::normalize_servers beside it), linked, session::ledger, session::ProviderAuth, wire::same_secret, driver::clip and failure::redact_text, desk::install_crypto_provider, computer::HOME_MOUNT, AuthStatus and import::Report, and voice's ledger, metering and spoken (contract, log and each other only). The three test rooms (CoreHandle, Quiet, Computered) keep implementing RoomHandle. Each moved module renames its own crate:: paths to the new crates; the core keeps `pub use` re-exports only while callers remain. The crate gets the workspace lints, a license and a crate doc saying what it owns and what it must never depend on (hotline-core, Tauri). docs/standards.md's module map, docs/security.md citations and AGENTS.md follow the moves. Name the crate in the ADR if hotline-room reads wrong (the validation looks for the moved files' absence from the core, not the name). In ADR 0007, list each moved file's old path and new path: epic-17's task-202 names room.rs and task-207 names voice/spoken.rs, and that list is how they find them.

## Log
- 2026-10-19T14:40:59.829Z: [claude] Lead: task-211 landed on release/0.41.0 (770640d9). The wire now receives the calls, the Remote and the links in a `wire::Parts` when it is built. Parts names core types, so it stays in the core; only RoomHandle and the plain pieces move. Run the full `cargo test -p hotline-core`, `make check` and `make precheck` in the foreground and to completion before ending your turn. task-211's first try ended uncommitted because it put the test run in the background.
