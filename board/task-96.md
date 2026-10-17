---
id: task-96
title: "Epic-10 end to end: a phone off-network pairs from the QR via the Public address and sees itself, the desk's kind and version, and the public endpoint marked"
column: review
position: 10
priority: high
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/mod.rs
  - docs/design.md
parentId: epic-10
dependsOn:
  - task-12
  - task-56
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/src/remote/tests.rs
      description: one end-to-end test over the wire covering epic-10's done-when on the desk side
  validation:
    commands:
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make check
  metrics:
    readyAt: "2026-10-17T05:44:24.740Z"
    pickedUpAt: "2026-10-17T07:21:53.483Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T07:30:17.967Z"
    duration: 504
createdAt: "2026-10-17T05:44:24.746Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T05:44:34.861Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T07:21:35.862Z"
    ended: "2026-10-17T07:30:30.021Z"
    outcome: review
assignee: claude
updatedAt: "2026-10-17T07:30:30.021Z"
---

## Description
Wires and proves the desk side of epic-10 against its "done when", after task-12 and task-56 have landed.

task-12 landed as 2850899b. Its wire fields: hello `publicEndpoint` (string, any seat, absent when no Public address; from `public_endpoint()` in `remote/mod.rs`), owner-only hello `deskKind` ("desktop" | "server", `desk_kind()`) and `version` (CARGO_PKG_VERSION), and `remote.devices` grants carry `current: true` on the caller's own grant only (absent otherwise). The QR fallback from task-56 should read the same `public_endpoint()`; if it doesn't, make it.

One over-the-wire test in `crates/hotline-core/src/remote/tests.rs` (TestPhone harness), standing in for a phone away from home:
1. Desk with Remote on and a Public address set. Make a pairing QR; parse its link the way a phone does.
2. The local `u=` endpoint is unreachable to this client; it pairs through the QR's public fallback endpoint (task-56).
3. Its hello carries `deskKind`, `version` and `publicEndpoint`, and `publicEndpoint` equals the fallback in the QR (task-12 and task-56 agree on one source).
4. `remote.devices` marks this phone's grant `current` and no other's.
5. With Listen on set to one specific address, the tunnel hint (RemoteSection) names that address and port; on all interfaces it names localhost.
Fix whatever seam the two children left between them (for example two different notions of "the public endpoint"). Update `docs/design.md` and `docs/` where they describe the pairing link and hello fields, so the docs say what is. The phone-side labels are hotline-mobile's board (task-25 there); note on epic-10 the exact wire field names the phone must read, including the QR's fallback parameter name.

## Log
- 2026-10-17T07:30:22.717Z: [claude] Done on branch dispatch/task-96 (a5adba5f). New test remote::tests::a_phone_away_from_home_pairs_through_the_public_address_and_sees_itself_and_the_desk: a real TCP tunnel on 127.0.0.1 is the Public address; the phone parses the QR, skips u= (this computer's own addresses count as not routing — inside one process they would route, so that part is simulated), pairs and connects through x=, and its hello has deskKind/version/publicEndpoint == x. remote.devices marks only its own grant current. The tunnel goes where the hint says: localhost:port on all IPs. On one address, localhost is refused and address:port works. No seam to fix: the QR's x and the hello's publicEndpoint both come from public_endpoint_of(); x is left out only when it equals u (a served desk). The test copies RemoteSection's hint rule (tunnel_hint); ui/tests/remote-section.test.tsx proves the window renders it. Docs: design.md (Public address paragraph), wire.md (u is the desk URL; x == publicEndpoint), security.md (QR x fallback + test). make check green (2009 passed). STILL UNWIRED: the Rust client (desks.rs) doesn't refresh PairedDesk.public from hello publicEndpoint, so desks paired before have public: None. Phone side (hotline-mobile task-25) not done.
