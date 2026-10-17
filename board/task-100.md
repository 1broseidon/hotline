---
id: task-100
title: "Desktop: a relay link pasted into \"Add a server\" says it added a relay and chooses it, instead of closing silently"
column: review
position: 3
description: |-
  Left by task-93 (7932bf89, on release/0.40.0): AddDesk ("Add a server") given a `hotline pair --link --role relay` link pairs it as a relay via `pairDeskByLink` (the shell keeps it in relays.json) and then closes with nothing switched and no message, so the person sees nothing happen. UI only; the shell and core already do the pairing.

  Make it unsurprising: when the pasted link pairs a relay (the link's `role:"relay"`, or the id not appearing in useDesks() but in `remote.relays`), AddDesk says "Added as a relay" and either chooses it with `remote.relay` when none is chosen, or points to Settings › Remote › Relay servers. Do the same thing the "Relay servers" fold in RemoteSection does; reuse its path rather than a second one. Delete any branch it replaces.

  Tests: extend ui/tests/add-desk.test.ts (or relay-picker.test.tsx) through the real Wire over a fake socket: a relay link in Add a server reports a relay and never lists a desk; a desk link is unchanged.
priority: medium
tags:
  - relay
  - ui
relatedFiles:
  - ui/src/components/RemoteSection.tsx
  - ui/tests/add-desk.test.ts
  - ui/tests/relay-picker.test.tsx
parentId: epic-11
contract:
  status: delivered
  validation:
    commands:
      - cd ui && bun install --frozen-lockfile && bun run typecheck && bun run test
  metrics:
    readyAt: "2026-10-17T09:03:23.679Z"
    pickedUpAt: "2026-10-17T09:03:50.248Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T09:07:11.231Z"
    duration: 201
createdAt: "2026-10-17T09:03:23.684Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T09:03:27.260Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T09:03:32.898Z"
assignee: claude
updatedAt: "2026-10-17T09:07:11.231Z"
---

## Description
Left by task-93 (7932bf89, on release/0.40.0): AddDesk ("Add a server") given a `hotline pair --link --role relay` link pairs it as a relay via `pairDeskByLink` (the shell keeps it in relays.json) and then closes with nothing switched and no message, so the person sees nothing happen. UI only; the shell and core already do the pairing.

Make it unsurprising: when the pasted link pairs a relay (the link's `role:"relay"`, or the id not appearing in useDesks() but in `remote.relays`), AddDesk says "Added as a relay" and either chooses it with `remote.relay` when none is chosen, or points to Settings › Remote › Relay servers. Do the same thing the "Relay servers" fold in RemoteSection does; reuse its path rather than a second one. Delete any branch it replaces.

Tests: extend ui/tests/add-desk.test.ts (or relay-picker.test.tsx) through the real Wire over a fake socket: a relay link in Add a server reports a relay and never lists a desk; a desk link is unchanged.

## Log
- 2026-10-17T09:07:10.898Z: [claude] Done on dispatch/task-100. RemoteSection exports keepPairedRelay(deskId): it always asks this computer's own core (wireFor of the local desk, even while a server's desk is active), and when the id is in remote.relays it chooses that relay only if none is chosen yet. AddDesk (link and SSH) and the Relay servers fold both use it. A relay in Add a server now shows "Added X as a relay" with either "Phones also reach this desk through it." or "Choose it under Settings › Remote › Relay.", stays open and never calls setActiveDesk. Behaviour change: the fold no longer replaces a relay that is already chosen. Also fixed the fold hint `--role relay` to the real CLI flag `hotline pair --relay --link`. Tests: add-desk.test.ts drives the real Wire over fake sockets (relay with none chosen, relay with one chosen from a server's desk, desk link unchanged); relay-picker.test.tsx now mocks the local desk's own Wire instead of `wire`. ui typecheck and all 420 tests pass.
