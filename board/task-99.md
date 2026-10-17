---
id: task-99
title: "Epic-11 end to end: on a relay-only server a desktop pairs as relay, a phone reaches it through, the desktop cannot prompt, read or configure the server, and a switch to agents,relay keeps the pairing"
column: todo
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
  status: ready
  validation:
    commands:
      - cargo test -p hotline-cli --test relay
      - cargo test --workspace --test '*'
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo fmt --all --check
  metrics:
    readyAt: "2026-10-17T07:46:38.560Z"
createdAt: "2026-10-17T07:46:38.565Z"
updatedAt: "2026-10-17T09:03:28.625Z"
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
