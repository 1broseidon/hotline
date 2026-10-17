---
id: task-99
title: "Epic-11 end to end: on a relay-only server a desktop pairs as relay, a phone reaches it through, the desktop cannot prompt, read or configure the server, and a switch to agents,relay keeps the pairing"
column: in-progress
position: 8
priority: high
tags:
  - relay
  - e2e
relatedFiles:
  - crates/hotline-cli/tests/relay.rs
  - crates/hotline-cli/tests/serve.rs
  - crates/hotline-cli/tests/pair.rs
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-core/src/remote/client.rs
parentId: epic-11
dependsOn:
  - task-93
  - task-94
contract:
  status: in_progress
  validation:
    commands:
      - cargo test -p hotline-cli --test relay
      - cargo test --workspace --test '*'
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo fmt --all --check
  metrics:
    readyAt: "2026-10-17T07:46:38.560Z"
    pickedUpAt: "2026-10-17T09:03:54.514Z"
    reworkCount: 0
createdAt: "2026-10-17T07:46:38.565Z"
updatedAt: "2026-10-17T09:03:54.514Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T09:03:26.679Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T09:03:28.625Z"
assignee: claude
---

## Description
Wire the epic's parts together and prove its "Done when" with real processes, in a new headless harness `crates/hotline-cli/tests/relay.rs` (the house idiom: start the real `hotline serve` binary and drive it over the wire, as `tests/serve.rs` and `tests/pair.rs` do). docs/adr/0006-relay-only-servers.md is the reference for names (`hotline pair --relay`, `serves`, `relayGrants`, `role_not_permitted`, `{"refused":"relay"}`). Fix whatever does not join up; say in notes what you had to change and why.

The harness, in one run on a temporary HOTLINE_DATA_DIR:
1. `hotline serve --capabilities relay` boots: no room files, no scheduler, no agent processes; `hotline status` answers with its capabilities.
2. A desktop core (a second Remote/desk in the test) pairs with it as relay from the link `hotline pair` prints there, and chooses it as its Relay.
3. A phone (test client) paired with that desktop reaches the desktop through the relay and gets its hello.
4. The desktop's relay grant is refused at `/v2` (prompt, read conversations), `/v2/computer` and the server's `/v2/link` with role_not_permitted, and `links`/`unlink`/`wire` on the door refuse.
5. A phone-shaped claim (no purpose) against the relay-only server makes no grant.
6. Stop, restart with `--capabilities agents,relay`: the desktop's relay grant still stands in without re-pairing; an owner's hello now says `serves: ["agents","relay"]`.

Kill only PIDs captured at spawn. The Pi itself is out of reach here: note in the log that the aarch64 Linux footprint is for George to confirm on hardware.

## Log
- 2026-10-17T09:03:23.942Z: [claude] [lead] From task-93/98: a relay's `serves` in the desktop's relays.json is what pairing/the move said and is never refreshed from the ready frame. In step 6 (switch to agents,relay) check what the desktop then believes; if it is stale in a way that matters for standing in, fix it in the core and say so; if it only affects display, note it for the UI rather than building it here. Wire.serves() now exists in the UI from the hello frame.
- 2026-10-17T09:14:12.793Z: [claude] Done on dispatch/task-99 (1be2eca9): new harness crates/hotline-cli/tests/relay.rs runs all six steps in one test with the real `hotline serve` binary, an in-process desktop Remote and phone Clients. Passed 5/5 runs; fmt, clippy -D warnings and `cargo test --workspace --test '*'` are green; docs/security.md matrix gains a row for it. NO CORE CHANGE NEEDED: everything already joined up. HOW: (4) on a relay-only server /v2, /v2/computer/<id>/ws and /v2/link are 404 before Noise (ADR: no route), so there is no role_not_permitted to see there; role_not_permitted is proved after the switch to agents,relay, where the same relay key gets it at all three doors. The core's Client never dials /v2/computer or /v2/link with a device key, so the test captures the key the desktop makes when it pairs (a recording SecretStore) and knocks with a hand-written Noise IK handshake (dev-deps snow, tokio-rustls); links/unlink/`wire welcome` refuse with "runs only a relay". (1) no-agent-processes = `pgrep -P <captured pid>` is empty, plus the top-level-files check. (6) after the restart the phone, still unable to reach the desktop's own address, gets the desktop's hello through the relay again, with no re-pairing; status says "runs agents, relay"; a fresh owner's hello says serves ["agents","relay"]. STALE SERVES (lead's question): relays.json still says serves ["relay"] after the switch, and standing in does not read it (relay.rs never looks at serves; the ready frame's serves is not kept), so it only affects display: the UI should refresh a relay's serves from the ready frame or not show it. Not built here. LEFT: the aarch64 Linux footprint on a Raspberry Pi is for George to confirm on hardware; it is out of reach here. Building needs .speech-engine (copied from the main checkout / scripts/fetch-speech-engine).
