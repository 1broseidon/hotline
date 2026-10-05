# Relay hardening (BRO-210)

Target: remote relay, package scope, Rust WebSockets/Noise. Caller: sdk-client.
Surface pattern: authenticated purpose-bound sockets, from remote/server.rs sealed_session.
Evidence: ADR 0002, remote/relay.rs Hub and stand, remote/server.rs Seat and relay_door,
remote/client.rs pair and handshake, remote/admission.rs accept/authenticate.

| Surface | Change | Risk | Compatibility | Artifact | Verification |
| --- | --- | --- | --- | --- | --- |
| /relay/{desk}/v2… | change | R2 | stable budgets, expiry and 429 desk_busy for visit rate | ADR 0002 | relay tests |
| /relay/accept/{cap} | remove | R4 | explicitly requested by BRO-213; replaced before relay release | ADR 0002 | old route refused |
| /v2/relay/accept | add | R1 | sealed IK, purpose relay-accept and capability; outer state discarded | ADR 0002 | ownership and end-to-end tests |
| control and relayed session capacity | change | R2 | hosting independent of direct seats; relayed devices share16/4 seats | ADR 0002 | admission and relay tests |
| Session.relay and Bridge.relay watches | add | R1 | additive Rust fields; no JSON wire change | serve.md | session/bridge/registry tests |
| pairing relay and authenticated hello endpoints | change | R2 | optional existing fields, learned only after authenticated Noise | ADR 0002 | discovery tests |

Obligations: single-use current unexpired device-owned capabilities; stable bounded budgets;
revocation and standing cancellation; bounded writes/closes; authenticated discovery persistence.
Expected edits: remote/{relay,server,client,admission,mod,bridge}.rs, remote tests,
hotline-app/src/desks.rs, ADR 0002 and serve.md. No file moves or deletions.
Verification: full cargo test -p hotline-core before commits; make check for final integration.

Final verification (2026-10-04):

- `RUST_MIN_STACK=33554432 CARGO_INCREMENTAL=0 cargo test -p hotline-core`: 1,577 passed, three ignored, no failures or filtered tests.
- `RUST_MIN_STACK=33554432 CARGO_INCREMENTAL=0 make check`: passed formatting, workspace clippy/tests, UI typecheck and 237 UI tests, and Python checks.
- The full core run regenerated `ui/src/generated/contract.ts` with no diff. Mobile source sync was unnecessary; native phone verification of relay path addresses remains required.

ADR 0002 is Accepted. It records the 1,024-entry stable-budget registry bound,
pending permits retained through sealed callback authentication and bounded
control replacement, and cancellation forwarded through the wire's own cleanup.
