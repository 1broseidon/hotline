---
id: task-92
title: "Core + CLI: serve --capabilities, a relay-only boot with no agent machinery, and mode switching"
column: backlog
position: 46
priority: high
tags:
  - relay
  - server
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.930Z"
dependsOn:
  - task-90
updatedAt: "2026-10-17T05:37:04.496Z"
---

## Description
Core + CLI: serve capabilities, and a relay-only boot.

- `hotline serve --capabilities relay|agents|agents,relay`, default `agents,relay`. It's persisted in the data dir, so a restart keeps the mode; a later flag switches it, keeping pairings.
- Relay-only boot:
  - no `Desk` room, scheduler, ACP drivers, computer runtime, models.json sync or provider credential store reads;
  - Remote opens on a lightweight host. Today `Remote::open_served_with_store` takes the full `Desk`, so introduce the narrow interface Remote actually needs (pairing, device grants, relay registry, status) and have `Desk` implement it.
  - The door still answers `hotline status` (mode, relay hosts, visits).
- Switching relay → agents,relay: the room is created on first agent boot; pairings and the relay registry carry over. agents → relay-only: refuse while teammates exist unless `--force`, which keeps their data untouched on disk.
- The hello carries `capabilities` (and `deskKind: server` from epic-10).

Tests:
- a relay-only boot opens no room files and starts no scheduler or agents, and relays a visit;
- a mode switch keeps grants;
- hello capabilities in each mode.

Measure the RSS and idle CPU of relay-only on aarch64 and note them in the docs.
