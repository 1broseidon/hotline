---
id: task-94
title: "Phone contract: a relay-only server's QR says it is a relay, a phone's pair claim there makes no grant, and wire.md tells hotline-mobile what to show"
column: todo
position: 8
priority: high
tags:
  - relay
  - phone
parentId: epic-11
createdAt: "2026-10-17T05:37:02.975Z"
dependsOn:
  - task-98
updatedAt: "2026-10-17T08:52:55.663Z"
relatedFiles:
  - crates/hotline-cli/src/remote.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-cli/tests/pair.rs
  - docs/wire.md
contract:
  status: ready
  validation:
    commands:
      - cargo test -p hotline-cli --test pair
      - cargo test -p hotline-core
      - cargo clippy --workspace --all-targets -- -D warnings
      - grep -qi 'relay' docs/wire.md && grep -qi 'capabilities' docs/wire.md
  metrics:
    readyAt: "2026-10-17T07:47:18.324Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T08:52:55.541Z"
  until: done
  tries: 0
assignee: claude
---

## Description
The phone never pairs with a relay-only server; it only dials through one. The phone's screens live in ../hotline-mobile and cannot be changed from this repository; this child checks the desk side the phone reads and documents it. Per docs/adr/0006-relay-only-servers.md, the server behaviour already exists from task-91/92: `r=relay` in the QR, `"role":"relay"` in the link, a claim without `purpose:"relay"` on a relay window answered `{"refused":"relay"}` with no grant, `serves` in the pair answer and hello, `role_not_permitted`.

- Prove through the real handler (crates/hotline-cli/tests/pair.rs) that an old-phone-shaped claim (no `purpose`) against a relay-only server's `hotline pair` makes no grant, leaves the window open, and gets `{"refused":"relay"}`. Add the test only if task-91/92 did not already cover it.
- A desk reached through a relay still shows "Reachable anywhere" as today: no change to what the desk sends.
- docs/wire.md: for the hotline-mobile side, the `r=relay` marker and what to say before dialing ("This is a relay. Pair your desk with it, then link the desk."), the `{"refused":"relay"}` answer, `serves`, `role_not_permitted`, and that `remote.devices` can now hold a `relay` row the phone must read as a role it cannot act on (never fail to decode).

Leave a note on this task giving the exact hotline-mobile task to file (title and what it must do); the lead passes it to George.
