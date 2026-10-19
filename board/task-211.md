---
id: task-211
title: "RoomHandle names nothing in the core: voice(), remote() and links() leave the trait, AuthStatus and import::Report become plain data"
column: in-progress
position: 12
description: |-
  ADR 0007 step 1, first half, in place inside hotline-core: no crate is made here (task-208 does that on top). RoomHandle (wire/room_handle.rs) has five signatures that name the core: voice() -> voice::Calls (line 51), remote() -> remote::Remote (55), links() -> linked::Links (60), a method returning driver::auth::AuthStatus (119) and import() returning import::Report (370). Cut them so the trait names only the contract and plain data:
  - voice(), remote() and links() leave the trait. The wire is handed the calls, the remote and the links when it is built, as the desk already holds them. Today about 21 call sites use them (wire/commands.rs, socket.rs, seat.rs, subscribe.rs, desk.rs, remote/link.rs, voice/dispatcher.rs, and tests in wire/tests, remote/tests, voice/tests); each reads from what the wire was built with instead. A room that has no voice/remote/links (the test rooms) is built with None, the same answer the default bodies give today.
  - AuthStatus and import::Report become plain data types that can move down with the trait (no methods that reach the core; whatever logic is on them stays where it is as free functions or in the core). If they belong in hotline-contract (they cross the wire), say why in the ADR; contract.ts must stay byte-identical, so if they are exported types, do not change their shape.
  - The three test rooms (CoreHandle and Quiet in wire/tests, Computered in remote/tests) keep implementing RoomHandle.
  No behaviour change, no new trait, no event hop (docs/standards.md). One line in ADR 0007 under step 1 saying it is done and how the wire now receives the three.
priority: high
tags:
  - crate-split
relatedFiles:
  - crates/hotline-core/src/wire/room_handle.rs
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/seat.rs
  - crates/hotline-core/src/wire/subscribe.rs
  - crates/hotline-core/src/desk.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/remote/link.rs
  - crates/hotline-core/src/driver/auth.rs
  - crates/hotline-core/src/import.rs
  - docs/adr/0007-split-hotline-core.md
parentId: epic-18
contract:
  status: in_progress
  validation:
    commands:
      - "! grep -nE 'fn (voice|remote|links)\\(&self\\)' crates/hotline-core/src/wire/room_handle.rs"
      - "! grep -nE 'crate::(voice|remote|linked|driver|import)::' crates/hotline-core/src/wire/room_handle.rs"
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Moves and re-plumbing only: no behaviour change on the wire, no new trait, no event hop"
    - ui/src/generated/contract.ts byte-identical after an unfiltered cargo test
    - Do not create the new crate here; task-208 does that
  metrics:
    readyAt: "2026-10-19T14:09:32.259Z"
    pickedUpAt: "2026-10-19T14:10:45.873Z"
    reworkCount: 0
createdAt: "2026-10-19T14:09:32.269Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T14:31:50.317Z"
  until: done
  tries: 1
assignee: claude
updatedAt: "2026-10-19T14:31:50.448Z"
---

## Description
ADR 0007 step 1, first half, in place inside hotline-core: no crate is made here (task-208 does that on top). RoomHandle (wire/room_handle.rs) has five signatures that name the core: voice() -> voice::Calls (line 51), remote() -> remote::Remote (55), links() -> linked::Links (60), a method returning driver::auth::AuthStatus (119) and import() returning import::Report (370). Cut them so the trait names only the contract and plain data:
- voice(), remote() and links() leave the trait. The wire is handed the calls, the remote and the links when it is built, as the desk already holds them. Today about 21 call sites use them (wire/commands.rs, socket.rs, seat.rs, subscribe.rs, desk.rs, remote/link.rs, voice/dispatcher.rs, and tests in wire/tests, remote/tests, voice/tests); each reads from what the wire was built with instead. A room that has no voice/remote/links (the test rooms) is built with None, the same answer the default bodies give today.
- AuthStatus and import::Report become plain data types that can move down with the trait (no methods that reach the core; whatever logic is on them stays where it is as free functions or in the core). If they belong in hotline-contract (they cross the wire), say why in the ADR; contract.ts must stay byte-identical, so if they are exported types, do not change their shape.
- The three test rooms (CoreHandle and Quiet in wire/tests, Computered in remote/tests) keep implementing RoomHandle.
No behaviour change, no new trait, no event hop (docs/standards.md). One line in ADR 0007 under step 1 saying it is done and how the wire now receives the three.

## Log
- 2026-10-19T14:31:49.451Z: [claude] Lead: try 1 ended before committing. It put the full test run in the background and ended its turn, which ends the session. The work is in the worktree uncommitted (61 files, +696/-407, ADR 0007 line included). Next try: do not redo it; review the diff and finish it. Run `cargo test -p hotline-core` unfiltered, then `make check`, `git diff --exit-code -- ui/src/generated/contract.ts` and `make precheck`, all in the foreground and to completion; never background a command and end the turn. Then commit with a one-line invariant message and deliver.
