---
id: task-91
title: "Core: a relay grant role that may only stand in and carry visits"
column: todo
position: 5
priority: high
tags:
  - relay
  - security
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.415Z"
dependsOn:
  - task-90
updatedAt: "2026-10-17T07:52:30.696Z"
relatedFiles:
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-core/src/remote/admission.rs
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/link.rs
  - crates/hotline-core/src/remote/tests/relay_tests.rs
  - crates/hotline-core/src/remote/tests/admission_tests.rs
  - crates/hotline-cli/src/main.rs
  - crates/hotline-cli/src/remote.rs
  - crates/hotline-cli/tests/pair.rs
  - ui/src/generated/contract.ts
  - docs/security.md
contract:
  status: ready
  validation:
    commands:
      - cargo test -p hotline-core
      - cargo test -p hotline-cli --test pair
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo fmt --all --check
      - grep -qi 'relay' docs/security.md
  metrics:
    readyAt: "2026-10-17T07:46:57.749Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T07:52:30.576Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Core: a `relay` grant role. Build exactly what docs/adr/0006-relay-only-servers.md decides (§ the relay grant role, § old-reader story, § a pair claim declares its purpose); where this text and the ADR differ, the ADR wins.

- `DeviceRole` gains `Relay`. Relay grants are saved in their own `relayGrants` list in `remote.json` (same `Grant` shape), never in `grants`; a key is in one list at most; the 16-device limit counts both.
- The owner-only check in `sealed_session` becomes a role table: `Wire` and `Computer` admit owner and companion; `Relay` admits owner and relay. `relay::accept` admits owner and relay. `Seat::for_phone` cannot yield a seat for a relay grant (make the type say so). A key refused for its role gets sealed `{"error":"role_not_permitted"}`, not `device_not_authorized`.
- The relay invitation: `hotline pair --relay` opens a pairing window with role relay (on a server that runs agents; task-92 makes it the only kind on a relay-only server). The pair claim gains optional `purpose`; a relay window admits only `purpose:"relay"`, an owner/companion window refuses it; a mismatch makes no grant, does not consume the window, answers `{"refused":"relay"}` / `{"refused":"device"}`, and counts toward the throttle. The relay pair answer is `{"role":"relay", deskName, deskId, serves}` (serves may be a fixed `["agents","relay"]` until task-92 adds capabilities). The QR says `r=relay` and the link payload `"role":"relay"`.
- `remote.devices` lists the role. Revocation works as today.
- Not here: capabilities, `serve.json`, `serves` in the hello and ready (task-92); the desktop's side of pairing as relay (task-98).

Tests through the real handler: a relay grant stands in and carries a phone visit end to end, and is refused at `/v2`, `/v2/computer` and the server's `/v2/link` with `role_not_permitted`; a phone claim on a relay window and a relay claim on an owner window make no grant; an old-shaped reader (the `Saved` struct without `relayGrants`) loads a file with relay grants and authenticates nothing from it. Add the relay role's row to docs/security.md's matrix and test index. Run `cargo test -p hotline-core` unfiltered before committing contract.ts.
