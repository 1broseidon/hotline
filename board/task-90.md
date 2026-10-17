---
id: task-90
title: "ADR 0006: server capabilities and the relay grant role"
column: in-progress
position: 4
priority: high
tags:
  - relay
  - adr
parentId: epic-11
createdAt: "2026-10-17T05:37:00.895Z"
updatedAt: "2026-10-17T07:47:42.492Z"
relatedFiles:
  - docs/adr/0006-relay-only-servers.md
  - docs/adr/0002-a-served-desk-relays-sealed-records.md
  - docs/adr/0003-linked-desks.md
  - docs/design.md
  - docs/security.md
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
contract:
  status: in_progress
  deliverables:
    - type: docs
      path: docs/adr/0006-relay-only-servers.md
      description: "the decision: capabilities, relay role, old-reader story, pair-claim purpose, QR marker, migration"
  validation:
    commands:
      - test -f docs/adr/0006-relay-only-servers.md
      - grep -qi 'old' docs/adr/0006-relay-only-servers.md && grep -qi 'capabilities' docs/adr/0006-relay-only-servers.md && grep -qi 'migrat' docs/adr/0006-relay-only-servers.md
      - grep -q '0006' docs/design.md
  constraints:
    - "No code changes: this child writes the decision only."
    - "docs/ holds only what is: write the ADR as the decision record, not as shipped behaviour in serve.md or security.md."
  metrics:
    readyAt: "2026-10-17T07:46:15.314Z"
    pickedUpAt: "2026-10-17T07:47:42.492Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T07:47:29.978Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T07:47:31.109Z"
assignee: claude
---

## Description
ADR 0006: what a server runs, and what a paired device may do there. Docs only, no code.

Write the decision before code: capabilities (agents, relay) as a serve-time mode, persisted in the data dir; the `relay` grant role and its exact allowed surface (relay control socket, accept, link-through), enforced where seats are admitted (`remote/server.rs` `sealed_session`, the owner-only check near :384, admission.rs); what a relay-only boot omits; how modes switch (pairings kept; agents need their own setup); discovery in the hello; migration of owner grants used only for relaying (an offer, never automatic, done by the device itself over its own grant).

Settle these, which later children rely on:
- The old-reader story. `RemoteDevice.role` is a serde enum inside `remote.json`: an old desk meeting `"relay"` fails to parse the whole file, not just that grant. Decide how a reader that does not know a role refuses only that grant (never treats it as owner, never loses the other pairings).
- How a relay-only server tells a desktop's relay pairing from a phone's: the pair claim carries a declared purpose, and a relay-only server refuses a claim that does not declare relay, so no grant is made (task-94 makes the phone side's contract).
- How the QR / pairing link of a relay-only server marks it a relay so a phone can explain before trying.

Reference ADR 0002 and ADR 0003, and docs/security.md (state default, grant source, enforcement points, tests, residual risk). Link the ADR from docs/design.md where ADRs are listed.
