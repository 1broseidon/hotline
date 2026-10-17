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
updatedAt: "2026-10-17T07:47:18.071Z"
relatedFiles:
  - crates/hotline-cli/src/remote.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-cli/tests/pair.rs
  - docs/wire.md
---

## Description
The phone never pairs with a relay-only server; it only dials through one. The phone's screens live in ../hotline-mobile and cannot be changed from this repository; this child makes and documents the desk side the phone reads.

- The pairing QR / link a relay-only server prints marks it a relay (the marker ADR 0006 chose), so a phone can explain before trying: "This is a relay. Pair your desk with it, then link the desk."
- A phone's pair claim against a relay-only server (one that does not declare relay, which task-98 enforces) gets the sealed refusal with a code the phone can show; no grant is made. Prove it here through the real handler if task-98 did not already.
- A desk reached through a relay still shows "Reachable anywhere" as today: no change to what the desk sends.
- docs/wire.md: the QR marker, the refusal code, and the hello's `capabilities`, written for the hotline-mobile side.

Leave a note on this task giving the exact hotline-mobile task to file (title and what it must do); the lead passes it to George.
