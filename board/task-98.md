---
id: task-98
title: "Core + app: a desktop pairs with a server as relay, keeps it as a relay not a desk, and can move its own owner grant to relay"
column: todo
position: 7
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
updatedAt: "2026-10-17T08:06:21.349Z"
---

## Description
The core and shell half of task-93, split out so the UI child is UI only. Build exactly what docs/adr/0006-relay-only-servers.md decides (§ old-reader story, last paragraph; § a pair claim declares its purpose; § migration); where this text and the ADR differ, the ADR wins. The server side (relay windows, claim purpose, `{"refused":…}`, `serves`) exists from task-91/92.

Left by task-91 (8319da55): `remote/client.rs` `pair` already sends `purpose:"relay"` for a relay payload and maps `{"refused":…}` to errors, but still returns a PairedDesk; `hotline-app/src/desks.rs` (the pair-from-link path, ~line 297) refuses a `DeviceRole::Relay` payload with "can't do yet". Replace that refusal with storing the relay as below (delete the guard, don't keep it), and make `client::pair`'s relay outcome a relay record rather than a desk.

Today a desktop reaches a server only as a remote desk (`hotline-app/src/desks.rs`, `remote/client.rs` PairedDesk, owner grant), and the Relay picker lists `useDesks()` of kind remote. A relay grant cannot open `/v2`, so:
- Pairing as relay: from a link/QR with `role:"relay"`, the desktop's claim sends `purpose:"relay"`, and stores the server as a relay (key, URL, name, `serves` from the pair answer) somewhere other than `desks.json` (an older window would list it as a desk and dial `/v2`): no bridge, no wire, not in the desk list. Say in the code where it lives and why.
- `remote.relay` can choose such a relay as well as an owner-paired desk; standing in uses the relay grant; `role_not_permitted` never marks the pairing revoked.
- Migration: one owner-seat command that moves the authenticated caller's own grant from `grants` to `relayGrants` (no device id argument), keeping id, key, name, pairing time, then ends its sessions like a revocation. The desktop then stands in again under the relay role and records the server as a relay. Relay → owner is not possible over the wire. The desktop knows from the hello (`serves`, teammates) whether to offer, so task-93 can decide.
- Commands the UI needs (add a relay from a link, list relays with `serves`, move to relay, remember a declined offer) with contract types in `ui/src/wire.ts`.

Tests through the real handler: a desktop pairs as relay and stands in, a phone visit reaches it; the relay is not in the desk list; owner → relay moves in place and the device is then refused `/v2` with role_not_permitted while still standing in; the move cannot target another device. Run `cargo test -p hotline-core` unfiltered before committing contract.ts.
