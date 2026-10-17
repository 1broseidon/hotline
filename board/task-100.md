---
id: task-100
title: "Desktop: a relay link pasted into \"Add a server\" says it added a relay and chooses it, instead of closing silently"
column: todo
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
  status: ready
  validation:
    commands:
      - cd ui && bun install --frozen-lockfile && bun run typecheck && bun run test
  metrics:
    readyAt: "2026-10-17T09:03:23.679Z"
createdAt: "2026-10-17T09:03:23.684Z"
---

## Description
Left by task-93 (7932bf89, on release/0.40.0): AddDesk ("Add a server") given a `hotline pair --link --role relay` link pairs it as a relay via `pairDeskByLink` (the shell keeps it in relays.json) and then closes with nothing switched and no message, so the person sees nothing happen. UI only; the shell and core already do the pairing.

Make it unsurprising: when the pasted link pairs a relay (the link's `role:"relay"`, or the id not appearing in useDesks() but in `remote.relays`), AddDesk says "Added as a relay" and either chooses it with `remote.relay` when none is chosen, or points to Settings › Remote › Relay servers. Do the same thing the "Relay servers" fold in RemoteSection does; reuse its path rather than a second one. Delete any branch it replaces.

Tests: extend ui/tests/add-desk.test.ts (or relay-picker.test.tsx) through the real Wire over a fake socket: a relay link in Add a server reports a relay and never lists a desk; a desk link is unchanged.
