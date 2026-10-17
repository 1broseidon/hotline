---
id: task-93
title: "Desktop: Add a relay, the Relay picker, and the offer to drop owner to relay"
column: backlog
position: 47
priority: high
tags:
  - relay
  - ui
parentId: epic-11
createdAt: "2026-10-17T05:37:02.457Z"
---

## Description
Desktop: add a relay, and use it with the least it needs.

- Settings › Remote: "Add a relay" pairs with a server as the `relay` role (`hotline pair --link --role relay` on the server prints the QR or link). The Relay picker lists relay-capable servers and labels relay-only ones.
- A relay-only server never shows in the team rail or desk list as a desk with teammates. Linked-desk UI skips it.
- Migration offer: a server this desktop is owner of, used as its Relay, whose hello says it runs agents but this desktop has no teammates there gets a one-time offer: "Use this server only as your relay? Your desktop will no longer be able to run or read anything on it." Accepting moves the grant to relay in place; declining is remembered. Never automatic.
- Tooltips 2-3 words (ui rules).

Tests over the wire, plus UI tests for the picker and the offer.
