---
id: task-92
title: "Core + CLI: serve --capabilities, a relay-only boot with no agent machinery, and mode switching"
column: review
position: 0
priority: high
tags:
  - relay
  - server
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.930Z"
dependsOn:
  - task-91
updatedAt: "2026-10-17T08:28:03.954Z"
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
contract:
  status: done
  validation:
    commands:
      - cargo test -p hotline-cli --test serve
      - cargo test -p hotline-core
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo fmt --all --check
      - grep -q -- '--capabilities' docs/serve.md
  metrics:
    readyAt: "2026-10-17T07:46:58.259Z"
    pickedUpAt: "2026-10-17T08:06:39.924Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T08:26:50.624Z"
    duration: 1211
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T08:06:22.222Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T08:06:23.490Z"
assignee: claude
---

## Description
Core + CLI: serve capabilities, and a relay-only boot. Build exactly what docs/adr/0006-relay-only-servers.md decides (§ what a server runs, § what a relay-only boot omits, § discovery, § switching modes); where this text and the ADR differ, the ADR wins. Builds on the relay role (task-91).

- `hotline serve --capabilities relay|agents|agents,relay`, default `agents,relay`, recorded in `serve.json` in the data dir (`{"capabilities":[...]}`); a flag that differs from the record replaces it and says so on stderr. `Purpose::of` answers a route only when its capability runs (else 404 before any Noise).
- Relay-only boot takes the room lock and claims the secret store, opens Remote (listener, admission, pairing, relay hub) and a loopback door, and opens nothing else: no room stream, vault, index, scheduler, sessions, drivers, MCP, skills, computer, voice, models, importer, provider keys; creates nothing under the room's directories. `Remote::open_served_with_store` takes `Arc<dyn RoomHandle>` (wire/mod.rs:81) and serve.rs passes the full `Desk`; give Remote the narrowest thing a relay-only boot needs rather than a stub Desk. Use the existing seam if it fits; add a trait only if it does not, and say why.
- The door answers `status` (with capabilities), `pair`, `devices`, `revoke`; `links`, `unlink`, `wire` refuse with a sentence saying the server runs only a relay. On a relay-only server `hotline pair` opens a relay window (its only kind); `--companion` is refused there.
- `serves` (not `capabilities`, which the hello already uses) goes in the pair answer, the relay's `ready` frame, and the v2 hello beside `deskKind` for an owner only.
- Switching: per the ADR, `remote.json` untouched, every grant keeps its role; relay → agents,relay opens the room fresh; agents,relay → relay leaves the room on disk unopened (no --force needed: nothing is lost); dropping relay makes `/v2/relay` 404.

Tests (extend `crates/hotline-cli/tests/serve.rs`, which runs the real binary): a relay-only boot creates no room files and starts no scheduler or agents, and relays a visit; a mode switch keeps grants and their roles; a relay-only server pairs only relay grants; `serves` in the pair answer, ready and hello in each mode; refused door commands.

Measure the RSS and idle CPU of relay-only on this machine (aarch64 macOS) and note the numbers and the platform in docs/serve.md along with capabilities; the Pi figure is for George to confirm on hardware.
