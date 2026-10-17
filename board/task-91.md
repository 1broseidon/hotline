---
id: task-91
title: "Core: a relay grant role that may only stand in and carry visits"
column: review
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
updatedAt: "2026-10-17T08:04:28.472Z"
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
  status: delivered
  validation:
    commands:
      - cargo test -p hotline-core
      - cargo test -p hotline-cli --test pair
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo fmt --all --check
      - grep -qi 'relay' docs/security.md
  metrics:
    readyAt: "2026-10-17T07:46:57.749Z"
    pickedUpAt: "2026-10-17T07:52:49.654Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T08:04:28.472Z"
    duration: 699
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T07:52:30.576Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T07:52:31.627Z"
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

## Log
- 2026-10-17T08:04:27.986Z: [claude] Done on dispatch/task-91 (8319da55). DeviceRole::Relay; relay grants in remote.json `relayGrants` (Saved::all_grants for lookups, limit, revoke, devices); role table `Purpose::admits` in remote/server.rs (Wire/Computer: owner+companion; Relay/RelayAccept: owner+relay), used by sealed_session and relay::accept; sealed ROLE_REFUSED `{"error":"role_not_permitted"}` also from the relay's /v2/link for a relay key; `Seat::for_phone(WireRole)` and seated_phone_v2 take a WireRole (no relay variant). Claim gains optional `purpose`; v2::Claimed::{Granted,Refused}; mismatch answers {"refused":"relay"|"device"}, keeps the window, counts toward throttle. Pair answer now carries `serves` for every role: fixed ["agents","relay"] on a served desk, ["agents"] on a desktop (Remote::serves in remote/mod.rs) — task-92 replaces it with capabilities. pairing_v2(Relay) is refused on a desktop's own Remote. CLI `hotline pair --relay` (refused with --companion); QR r=relay. core client::pair sends purpose:"relay" for a relay payload and reads `refused`. FOR TASK-98: hotline-app desks.rs `claim` currently REFUSES relay payloads so a relay never lands in desks.json — replace that guard with the real relay storage. client.rs still maps role_not_permitted to OpenError::Unreachable("refused this session") (not Revoked); task-98 may want a distinct variant. RemoteSection labels relay devices "Relay only". docs: security.md matrix row + "The relay grant role" section; wire.md pair claim/answer/role_not_permitted updated; serve.md NOT updated (no desktop can claim a relay invite yet — task-98/95). Not done here: capabilities, serve.json, serves in hello/ready, narrowing command.
