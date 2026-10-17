---
id: task-56
title: Pairing QR carries the Public address as a fallback endpoint; tunnel hint names the real listen address
column: todo
position: 1
priority: medium
tags:
  - remote
  - pairing
createdAt: "2026-10-14T19:29:48.828Z"
updatedAt: "2026-10-17T05:24:54.353Z"
parentId: epic-10
---

## Description
Audit 2026-10-17, desk 0.39.0:

- `remote/v2.rs:117` takes `s.endpoints.first()` for the QR's `u=` (used at :124, :136). `public_url` is never read in `pairing_v2`. A relay goes in as `a=` (:148-150); `r=` is the role.
- The local-first order is deliberate (`remote/mod.rs:374-375`) so the QR pairs on the network the phone is standing on. Keep it.
- `ui/src/components/RemoteSection.tsx:34` always says "Point it at https://localhost:{port}", with the port from the first endpoint (:24). That's wrong when Listen on is one specific address.

Change:
- The QR carries the Public address as an extra endpoint the phone tries after `u=` (the same fallback it already has for `a=`). Choose a param name that doesn't clash with `r=`/`a=`, and keep the QR scannable (length).
- Old phones ignore the unknown param and keep pairing locally.
- The tunnel hint names the real listen address and port (the chosen address, or localhost when listening on all).

Tests:
- the pairing link with and without a Public address and with a relay;
- a Rust client pairs via the public endpoint when the local one is unreachable;
- the hint follows Listen on.
- Phone: hotline-mobile parses and tries the new endpoint. Coordinate with that board.
