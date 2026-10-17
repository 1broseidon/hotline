---
id: task-92
title: "Core + CLI: serve --capabilities, a relay-only boot with no agent machinery, and mode switching"
column: todo
position: 6
priority: high
tags:
  - relay
  - server
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.930Z"
dependsOn:
  - task-91
updatedAt: "2026-10-17T07:46:58.008Z"
relatedFiles:
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-cli/src/main.rs
  - crates/hotline-cli/src/door.rs
  - crates/hotline-cli/src/remote.rs
  - crates/hotline-cli/tests/serve.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/served.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-core/src/wire/mod.rs
  - crates/hotline-core/src/desk.rs
  - docs/serve.md
---

## Description
Core + CLI: serve capabilities, and a relay-only boot. Follows ADR 0006 (task-90); builds on the relay role (task-91). Waits on task-91 because both change remote/mod.rs, remote/server.rs and hotline-cli/src/main.rs, and a relay-only server needs the relay role to exist.

- `hotline serve --capabilities relay|agents|agents,relay`, default `agents,relay`. It's persisted in the data dir, so a restart keeps the mode; a later flag switches it, keeping pairings.
- Relay-only boot:
  - no `Desk` room, scheduler, ACP drivers, computer runtime, models.json sync or provider credential store reads;
  - Remote opens on a lightweight host. `Remote::open_served_with_store` takes a `Arc<dyn RoomHandle>` (wire/mod.rs:81) and serve.rs passes the full `Desk`; give Remote the narrowest thing it needs on a relay-only boot (pairing, device grants, relay registry, status) rather than a stub Desk. Use the existing `RoomHandle` seam if it fits; add a new trait only if it does not, and say why.
  - The door still answers `hotline status` (mode, relay hosts, visits).
  - A relay-only server makes only relay grants, whatever role `hotline pair` asks for (moved here from task-91).
- Switching relay → agents,relay: the room is created on first agent boot; pairings and the relay registry carry over. agents → relay-only: refuse while teammates exist unless `--force`, which keeps their data untouched on disk.
- The hello carries `capabilities` beside `deskKind` (wire/mod.rs ~:1168).

Tests (extend `crates/hotline-cli/tests/serve.rs`, the harness that runs the real binary):
- a relay-only boot opens no room files and starts no scheduler or agents, and relays a visit;
- a mode switch keeps grants; agents → relay refused with teammates, allowed with --force;
- a relay-only server pairs only relay grants;
- hello capabilities in each mode.

Measure the RSS and idle CPU of relay-only on this machine (aarch64 macOS) and note the numbers and the platform in docs/serve.md; the Pi figure is for George to confirm on hardware.
