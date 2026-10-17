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
updatedAt: "2026-10-17T07:47:17.559Z"
relatedFiles:
  - ui/src/components/RemoteSection.tsx
  - ui/src/desks.ts
  - ui/src/wire.ts
  - ui/tests/relay-picker.test.tsx
  - ui/tests/add-desk.test.ts
---

## Description
Desktop UI: add a relay, and use it with the least it needs. UI only: the commands, the relay store and the owner → relay move are built in task-98; use them, don't rebuild them.

- Settings › Remote (`RemoteSection.tsx`): "Add a relay" pairs with a server as the `relay` role from the link `hotline pair --link --role relay` prints. The Relay picker lists relays and relay-capable desks together and labels relay-only ones.
- A relay-only server never shows in the team rail or desk list as a desk with teammates. Linked-desk UI skips it.
- Migration offer: a server this desktop is owner of, used as its Relay, whose hello says it runs agents but this desktop has no teammates there, gets a one-time offer: "Use this server only as your relay? Your desktop will no longer be able to run or read anything on it." Accepting calls task-98's move; declining is remembered. Never automatic.
- Tooltips 2-3 words (ui rules).

Tests: UI tests (`bun test`) for the picker (relay-only labelled, not in the desk list) and the offer (shown once under the conditions above, never otherwise, decline remembered).
