---
id: task-90
title: "ADR 0006: server capabilities and the relay grant role"
column: todo
position: 4
priority: high
tags:
  - relay
  - adr
parentId: epic-11
createdAt: "2026-10-17T05:37:00.895Z"
updatedAt: "2026-10-17T05:42:59.020Z"
---

## Description
ADR 0006: what a server runs, and what a paired device may do there.

Write the decision before code: capabilities (agents, relay) as a serve-time mode, persisted in the data dir; the `relay` grant role and its exact allowed surface (relay control socket, accept, link-through), enforced where seats are admitted (`remote/server.rs` around :384, admission.rs); what a relay-only boot omits; how modes switch (pairings kept; agents need their own setup); discovery in the hello; migration of owner grants used only for relaying (an offer, never automatic). Reference ADR 0002 (a served desk relays sealed records) and ADR 0003 (linked desks), and docs/security.md (the capability change follows it: default, grant source, enforcement points, tests, residual risk).
