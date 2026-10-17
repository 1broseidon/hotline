---
id: task-99
title: "Epic-11 end to end: on a relay-only server a desktop pairs as relay, a phone reaches it through, the desktop cannot prompt, read or configure the server, and a switch to agents,relay keeps the pairing"
column: todo
position: 8
description: |-
  Wire the epic's parts together and prove its "Done when" with real processes, in a new headless harness `crates/hotline-cli/tests/relay.rs` (the house idiom: start the real `hotline serve` binary and drive it over the wire, as `tests/serve.rs` and `tests/pair.rs` do). Fix whatever does not join up; say in notes what you had to change and why.

  The harness, in one run on a temporary HOTLINE_DATA_DIR:
  1. `hotline serve --capabilities relay` boots: no room files, no scheduler, no agent processes; `hotline status` answers with mode and relay hosts.
  2. A desktop core (a second Remote/desk in the test) pairs with it as relay via `hotline pair --link --role relay` and chooses it as its Relay.
  3. A phone (test client) paired with that desktop reaches the desktop through the relay and gets its hello.
  4. The desktop's relay grant is refused `/v2` wire (prompt, read conversations), the computer paths, settings commands and the door: each refused at admission.
  5. A phone's undeclared pair claim against the relay-only server makes no grant.
  6. Stop, restart with `--capabilities agents,relay`: the desktop's relay grant still stands in without re-pairing, the hello now says agents.

  Kill only PIDs captured at spawn. The Pi itself is out of reach here: note in the log that the aarch64 Linux footprint is for George to confirm on hardware.
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
---

## Description
Wire the epic's parts together and prove its "Done when" with real processes, in a new headless harness `crates/hotline-cli/tests/relay.rs` (the house idiom: start the real `hotline serve` binary and drive it over the wire, as `tests/serve.rs` and `tests/pair.rs` do). Fix whatever does not join up; say in notes what you had to change and why.

The harness, in one run on a temporary HOTLINE_DATA_DIR:
1. `hotline serve --capabilities relay` boots: no room files, no scheduler, no agent processes; `hotline status` answers with mode and relay hosts.
2. A desktop core (a second Remote/desk in the test) pairs with it as relay via `hotline pair --link --role relay` and chooses it as its Relay.
3. A phone (test client) paired with that desktop reaches the desktop through the relay and gets its hello.
4. The desktop's relay grant is refused `/v2` wire (prompt, read conversations), the computer paths, settings commands and the door: each refused at admission.
5. A phone's undeclared pair claim against the relay-only server makes no grant.
6. Stop, restart with `--capabilities agents,relay`: the desktop's relay grant still stands in without re-pairing, the hello now says agents.

Kill only PIDs captured at spawn. The Pi itself is out of reach here: note in the log that the aarch64 Linux footprint is for George to confirm on hardware.
