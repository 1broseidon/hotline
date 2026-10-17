---
id: task-96
title: "Epic-10 end to end: a phone off-network pairs from the QR via the Public address and sees itself, the desk's kind and version, and the public endpoint marked"
column: todo
position: 10
description: |-
  Wires and proves the desk side of epic-10 against its "done when", after task-12 and task-56 have landed.

  One over-the-wire test in `crates/hotline-core/src/remote/tests.rs` (TestPhone harness), standing in for a phone away from home:
  1. Desk with Remote on and a Public address set. Make a pairing QR; parse its link the way a phone does.
  2. The local `u=` endpoint is unreachable to this client; it pairs through the QR's public fallback endpoint (task-56).
  3. Its hello carries `deskKind`, `version` and marks the public endpoint, and the marked endpoint equals the one in the QR (task-12 and task-56 agree on one source).
  4. `remote.devices` marks this phone's grant `current` and no other's.
  5. With Listen on set to one specific address, the tunnel hint (RemoteSection) names that address and port; on all interfaces it names localhost.
  Fix whatever seam the two children left between them (for example two different notions of "the public endpoint"). Update `docs/design.md` and `docs/` where they describe the pairing link and hello fields, so the docs say what is. The phone-side labels are hotline-mobile's board (task-25 there); note on epic-10 the exact wire field names the phone must read.
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
  status: ready
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
createdAt: "2026-10-17T05:44:24.746Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T05:44:34.861Z"
  until: done
  tries: 0
assignee: claude
updatedAt: "2026-10-17T05:44:34.984Z"
---

## Description
Wires and proves the desk side of epic-10 against its "done when", after task-12 and task-56 have landed.

One over-the-wire test in `crates/hotline-core/src/remote/tests.rs` (TestPhone harness), standing in for a phone away from home:
1. Desk with Remote on and a Public address set. Make a pairing QR; parse its link the way a phone does.
2. The local `u=` endpoint is unreachable to this client; it pairs through the QR's public fallback endpoint (task-56).
3. Its hello carries `deskKind`, `version` and marks the public endpoint, and the marked endpoint equals the one in the QR (task-12 and task-56 agree on one source).
4. `remote.devices` marks this phone's grant `current` and no other's.
5. With Listen on set to one specific address, the tunnel hint (RemoteSection) names that address and port; on all interfaces it names localhost.
Fix whatever seam the two children left between them (for example two different notions of "the public endpoint"). Update `docs/design.md` and `docs/` where they describe the pairing link and hello fields, so the docs say what is. The phone-side labels are hotline-mobile's board (task-25 there); note on epic-10 the exact wire field names the phone must read.
