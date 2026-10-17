---
id: task-93
title: "Desktop: Add a relay, the Relay picker, and the offer to drop owner to relay"
column: todo
position: 7
priority: high
tags:
  - relay
  - ui
parentId: epic-11
createdAt: "2026-10-17T05:37:02.457Z"
dependsOn:
  - task-98
updatedAt: "2026-10-17T08:52:55.057Z"
relatedFiles:
  - ui/src/components/RemoteSection.tsx
  - ui/src/desks.ts
  - ui/src/wire.ts
  - ui/tests/relay-picker.test.tsx
  - ui/tests/add-desk.test.ts
contract:
  status: ready
  validation:
    commands:
      - cd ui && bun install --frozen-lockfile && bun run typecheck && bun run test
      - ls ui/tests | grep -qi relay
  metrics:
    readyAt: "2026-10-17T07:47:17.813Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T08:52:54.926Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Desktop UI: add a relay, and use it with the least it needs. UI only: the commands, the relay store and the owner → relay move are built in task-98; use them, don't rebuild them.

What task-98 left you (landed on release/0.40.0, 4d182ce0):
- Pairing: `desk_pair_link` (native.ts) already accepts a `role:"relay"` link and keeps it in relays.json (not a desk, no bridge, not listed); it answers the id string for either kind.
- Listing relays: wire `remote.relays {}` → `PairedRelay[]` (`{deskId,name,url,deskKey,public?,serves}`, in the generated contract).
- Tauri commands wrapped in native.ts: `moveDeskToRelay(deskId)` → PairedRelay, `declineRelay(deskId)` (sets `PairedDesk.relayDeclined`), `forgetRelay(deskId)`.
- `remote.relay` already accepts either an owner-paired desk or a relay id.
- Offer the move only when the desk's hello `serves` includes "relay", this desktop has no teammates there, and `!relayDeclined`.
- Known gaps you must handle in the UI: `forgetRelay` does not clear the core's saved `remote.relay` choice, so when forgetting the relay currently chosen, clear the choice too (or say it in the UI); a relay's `serves` is what pairing/the move said, not refreshed.

- Settings › Remote (`RemoteSection.tsx`): "Add a relay" pairs with a server as the `relay` role from the link `hotline pair --link --role relay` prints. The Relay picker lists `useDesks()` remote desks and `remote.relays` together and labels relay-only ones. A relay can be forgotten.
- A relay-only server never shows in the team rail or desk list as a desk with teammates. Linked-desk UI skips it.
- Migration offer: a server this desktop is owner of, used as its Relay, whose hello says it runs agents but this desktop has no teammates there, gets a one-time offer: "Use this server only as your relay? Your desktop will no longer be able to run or read anything on it." Accepting calls `moveDeskToRelay`; declining calls `declineRelay` and is remembered. Never automatic.
- Tooltips 2-3 words (ui rules).

Tests: UI tests (`bun test`) for the picker (relay-only labelled, not in the desk list) and the offer (shown once under the conditions above, never otherwise, decline remembered).
