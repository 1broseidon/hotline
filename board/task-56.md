---
id: task-56
title: Pairing QR carries the Public address as a fallback endpoint; tunnel hint names the real listen address
column: in-progress
position: 1
priority: medium
tags:
  - remote
  - pairing
createdAt: "2026-10-14T19:29:48.828Z"
updatedAt: "2026-10-17T06:37:19.151Z"
parentId: epic-10
relatedFiles:
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/tests.rs
  - ui/src/components/RemoteSection.tsx
  - crates/hotline-core/src/contract.rs
  - ui/src/generated/contract.ts
dependsOn:
  - task-12
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/src/remote/tests.rs
      description: pairing link with/without Public address and relay; a client pairs via the public endpoint when the local one is unreachable
  validation:
    commands:
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make check
  constraints:
    - u= stays the first local endpoint
    - New param must not clash with r=/a=; old phones ignore it
    - Use task-12's public-endpoint marking rather than a second source of truth
  metrics:
    readyAt: "2026-10-17T05:44:15.028Z"
    pickedUpAt: "2026-10-17T06:37:19.151Z"
    reworkCount: 1
    deliveredAt: "2026-10-17T06:34:37.005Z"
    duration: 674
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T05:44:34.344Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T06:36:54.632Z"
assignee: claude
---

## Description
Audit 2026-10-17, desk 0.39.0:

- `remote/v2.rs:117` takes `s.endpoints.first()` for the QR's `u=` (used at :124, :136). `public_url` is never read in `pairing_v2`. A relay goes in as `a=` (:148-150); `r=` is the role.
- The local-first order is deliberate (`remote/mod.rs:374-375`) so the QR pairs on the network the phone is standing on. Keep it.
- `ui/src/components/RemoteSection.tsx:34` always says "Point it at https://localhost:{port}", with the port from the first endpoint (:24). That's wrong when Listen on is one specific address.

From task-12 (landed): the public address has one source now, `Remote::public_endpoint()` in `remote/mod.rs` (private `fn`; a served desk gives `options.public_url`, a desktop gives `saved.public_url`, None while there are no endpoints). The hello exposes it as `publicEndpoint`. Read the QR's fallback from that helper (call it under the same lock, or factor it so `pairing_v2` doesn't take the state lock twice); do not read `saved.public_url` a second way. Skip the fallback param when it equals `u=`.

Change:
- The QR carries the Public address as an extra endpoint the phone tries after `u=` (the same fallback it already has for `a=`). Choose a param name that doesn't clash with `r=`/`a=`, and keep the QR scannable (length).
- Old phones ignore the unknown param and keep pairing locally.
- The tunnel hint names the real listen address and port (the chosen address, or localhost when listening on all).

Tests:
- the pairing link with and without a Public address and with a relay;
- a Rust client pairs via the public endpoint when the local one is unreachable;
- the hint follows Listen on.
- Phone: hotline-mobile parses and tries the new endpoint. Coordinate with that board.

## Log
- 2026-10-17T06:23:40.465Z: [claude] task-12 landed (2850899b on release/0.40.0): the one source for the public endpoint is `public_endpoint()` in remote/mod.rs (served options' public_url, else saved.public_url), sent as hello `publicEndpoint`. The QR's fallback endpoint must come from that same function, not a second reading of saved.public_url.
- 2026-10-17T06:34:40.829Z: [claude] Done on branch dispatch/task-56 (82d97b26). QR keeps u= = first local endpoint and adds x=<Public address> (skipped when equal to u= or unset; a= relay unchanged). Source is Remote::public_endpoint_of(&Live), factored from task-12's public_endpoint() so pairing_v2 takes the lock once. PairingPayload and client::PairedDesk gain optional `public`; the Rust client (pair and every later open) tries url → public → relay. Tunnel hint: localhost:port when Listen on is all, else the chosen endpoint. Tests: remote/tests.rs the_pairing_qr_offers_the_public_address_after_the_local_one_and_before_the_relay, rust_client_pairs_and_connects_through_the_public_address_when_its_own_does_not_answer; ui/tests/remote-section "the tunnel hint names the address Remote listens on". docs/wire.md + serve.md updated; make check green. UNWIRED: hotline-mobile must parse x= and try it after u= and before a= (not done here, mobile board read-only from this repo); the Rust client does not learn/refresh `public` from the hello's publicEndpoint after pairing (only relay is discovered), and desks paired before this have public: None.
- 2026-10-17T06:36:50.675Z: Landing sent back (1 of 1): with release/0.40.0 brought in, `make check` failed: failures: ⏎ remote::tests::sealed_network::cancelled_expired_and_wrong_secret_pairings_never_grant ⏎ test result: FAILED. 1853 passed; 1 failed; 9 ignored; 0 measured; 0 filtered out; finished in 35.76s ⏎ error: test failed, to rerun pass `-p hotline-core --lib` ⏎ make: *** [check] Error 101. Fix it on this branch, commit, and run the checks again.
- 2026-10-17T06:45:04.623Z: [claude] Landing fix on dispatch/task-56 (a1cd577d): sealed_network::cancelled_expired_and_wrong_secret_pairings_never_grant is untouched by this branch and passed alone, in 3 full lib runs and 48 parallel runs under a concurrent full suite; I could not reproduce it. Every refusal in it closes promptly, so the only way it can panic is the sealed helpers' 3s timeout().unwrap() (exchange/read_sealed/closed) under the reviewer's load. Widened that one budget to 15s (const ANSWER); a hang still fails. make check green (1854 passed). Root cause is inferred, not observed: if it recurs, capture the panic line. Still UNWIRED from before: hotline-mobile must parse x= and try it after u=, before a=; the Rust client does not refresh `public` from hello publicEndpoint; desks paired earlier have public: None.
