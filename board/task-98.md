---
id: task-98
title: "Core + app: a desktop pairs with a server as relay, keeps it as a relay not a desk, and can move its own owner grant to relay"
column: todo
position: 7
description: |-
  The core and shell half of task-93, split out so the UI child is UI only. Follows ADR 0006 (task-90).

  Today a desktop reaches a server only as a remote desk (`hotline-app/src/desks.rs`, `remote/client.rs` PairedDesk, owner grant), and the Relay picker lists `useDesks()` of kind remote. A relay grant cannot open `/v2`, so a relay must be kept apart from desks:
  - Pairing as relay: the desktop's pair claim declares the relay purpose (ADR 0006); the server makes a `relay` grant. The desktop stores it as a relay (server key, URL, name, capabilities), never as a desk: no bridge, no wire, not in the desk list.
  - `remote.relay` can choose such a relay as well as an owner-paired desk; standing in uses the relay grant.
  - A relay-only server refuses a pair claim that does not declare relay: no grant is made, and the sealed refusal names why (task-94 documents it for the phone).
  - Migration: a device may move its own grant owner → relay in place over its own sealed session (one command, only on its own grant, never the reverse). The desktop learns from the hello whether the server runs agents and whether it has teammates there, so the UI (task-93) can decide to offer.
  - Commands the UI needs (add a relay from a pairing link, list relays with capabilities, move to relay, remember a declined offer) exist with their contract types in `ui/src/wire.ts`.

  Tests through the real handler: a desktop pairs as relay and stands in, a phone visit reaches it; the relay is not a desk; a relay-only server refuses an undeclared pair claim and makes no grant; owner → relay moves in place and the device is then refused `/v2`; relay → owner is refused. Run `cargo test -p hotline-core` unfiltered before committing contract.ts.
priority: high
tags:
  - relay
  - core
  - app
relatedFiles:
  - crates/hotline-core/src/remote/client.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-core/src/remote/relay.rs
  - crates/hotline-core/src/remote/tests/relay_tests.rs
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/contract.rs
  - crates/hotline-app/src/desks.rs
  - ui/src/wire.ts
  - ui/src/generated/contract.ts
parentId: epic-11
dependsOn:
  - task-92
contract:
  status: ready
  validation:
    commands:
      - cargo test -p hotline-core
      - cargo test -p hotline-app
      - cargo clippy --workspace --all-targets -- -D warnings
      - cd ui && bun run typecheck
  metrics:
    readyAt: "2026-10-17T07:46:27.374Z"
createdAt: "2026-10-17T07:46:27.380Z"
---

## Description
The core and shell half of task-93, split out so the UI child is UI only. Follows ADR 0006 (task-90).

Today a desktop reaches a server only as a remote desk (`hotline-app/src/desks.rs`, `remote/client.rs` PairedDesk, owner grant), and the Relay picker lists `useDesks()` of kind remote. A relay grant cannot open `/v2`, so a relay must be kept apart from desks:
- Pairing as relay: the desktop's pair claim declares the relay purpose (ADR 0006); the server makes a `relay` grant. The desktop stores it as a relay (server key, URL, name, capabilities), never as a desk: no bridge, no wire, not in the desk list.
- `remote.relay` can choose such a relay as well as an owner-paired desk; standing in uses the relay grant.
- A relay-only server refuses a pair claim that does not declare relay: no grant is made, and the sealed refusal names why (task-94 documents it for the phone).
- Migration: a device may move its own grant owner → relay in place over its own sealed session (one command, only on its own grant, never the reverse). The desktop learns from the hello whether the server runs agents and whether it has teammates there, so the UI (task-93) can decide to offer.
- Commands the UI needs (add a relay from a pairing link, list relays with capabilities, move to relay, remember a declined offer) exist with their contract types in `ui/src/wire.ts`.

Tests through the real handler: a desktop pairs as relay and stands in, a phone visit reaches it; the relay is not a desk; a relay-only server refuses an undeclared pair claim and makes no grant; owner → relay moves in place and the device is then refused `/v2`; relay → owner is refused. Run `cargo test -p hotline-core` unfiltered before committing contract.ts.
