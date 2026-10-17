---
id: task-56
title: Pairing QR carries the Public address as a fallback endpoint; tunnel hint names the real listen address
column: in-progress
position: 1
priority: medium
tags:
  - remote
  - pairing
createdAt: "2026-10-14T19:29:48.828Z"
updatedAt: "2026-10-17T06:23:23.098Z"
parentId: epic-10
relatedFiles:
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/tests.rs
  - ui/src/components/RemoteSection.tsx
  - crates/hotline-core/src/contract.rs
  - ui/src/generated/contract.ts
dependsOn:
  - task-12
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/src/remote/tests.rs
      description: pairing link with/without Public address and relay; a client pairs via the public endpoint when the local one is unreachable
  validation:
    commands:
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make check
  constraints:
    - u= stays the first local endpoint
    - New param must not clash with r=/a=; old phones ignore it
    - Use task-12's public-endpoint marking rather than a second source of truth
  metrics:
    readyAt: "2026-10-17T05:44:15.028Z"
    pickedUpAt: "2026-10-17T06:23:23.098Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T05:44:34.344Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T06:22:26.794Z"
assignee: claude
---

## Description
Audit 2026-10-17, desk 0.39.0:

- `remote/v2.rs:117` takes `s.endpoints.first()` for the QR's `u=` (used at :124, :136). `public_url` is never read in `pairing_v2`. A relay goes in as `a=` (:148-150); `r=` is the role.
- The local-first order is deliberate (`remote/mod.rs:374-375`) so the QR pairs on the network the phone is standing on. Keep it.
- `ui/src/components/RemoteSection.tsx:34` always says "Point it at https://localhost:{port}", with the port from the first endpoint (:24). That's wrong when Listen on is one specific address.

From task-12 (landed): the public address has one source now, `Remote::public_endpoint()` in `remote/mod.rs` (private `fn`; a served desk gives `options.public_url`, a desktop gives `saved.public_url`, None while there are no endpoints). The hello exposes it as `publicEndpoint`. Read the QR's fallback from that helper (call it under the same lock, or factor it so `pairing_v2` doesn't take the state lock twice); do not read `saved.public_url` a second way. Skip the fallback param when it equals `u=`.

Change:
- The QR carries the Public address as an extra endpoint the phone tries after `u=` (the same fallback it already has for `a=`). Choose a param name that doesn't clash with `r=`/`a=`, and keep the QR scannable (length).
- Old phones ignore the unknown param and keep pairing locally.
- The tunnel hint names the real listen address and port (the chosen address, or localhost when listening on all).

Tests:
- the pairing link with and without a Public address and with a relay;
- a Rust client pairs via the public endpoint when the local one is unreachable;
- the hint follows Listen on.
- Phone: hotline-mobile parses and tries the new endpoint. Coordinate with that board.
