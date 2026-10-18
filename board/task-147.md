---
id: task-147
title: "P2-38 (D): The remote harness connects over loopback"
column: in-progress
position: 36
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:00.111Z"
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/network.rs
  - crates/hotline-core/src/remote.rs
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-cli/tests/serve.rs
  - crates/hotline-cli/tests/relay.rs
  - crates/hotline-cli/tests/pair.rs
updatedAt: "2026-10-18T05:21:52.050Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T05:21:52.050Z"
    reworkCount: 0
dependsOn:
  - task-146
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:07.611Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:21:26.528Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 38 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The remote harness connects over loopback.** The interface test keeps real addresses. Then let `Remote` and `ServeOptions` take a bound listener in tests, ending the free-port race

- Source: tests-and-docs T1, T11, task 3
- Files: `remote/tests.rs:62-163`, `remote/network.rs`, `crates/hotline-cli/tests/{serve,relay,pair}.rs`
- Collides with: 25, 37
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:43:30.109Z: [claude] Done on dispatch/task-147 (1c3ebc92). New public constructors Remote::open_on(listener, …) and Remote::open_served_on(listener, …, options). Each stores a network::Bound: the first start serves on the socket the caller bound, and a start after a stop binds the same address again, because a stopped Remote has to refuse connections and only a closed socket does. A desktop opened on a listener advertises only that listener's address, and its cert is made for that address. Harness::new()/with_computer() now use a 127.0.0.1 socket the test binds itself, and Harness::reopen(store) replaces the reopen sites (legacy, identity x2, sealed_network). link.rs desks, the relay/admission served desks, the PEM served_listener test, and the in-process desktop() helpers in hotline-cli tests/serve.rs and tests/relay.rs all take a bound listener now. Only three tests still use real interfaces, through Harness::on_interfaces(): all_interfaces_…, a_saved_loopback_selection_…, a_phone_away_from_home_…. I checked this by making addresses() return nothing: exactly those three failed. The interface test now checks the narrowed listener refuses only addresses that answered earlier. Left unwired: the CLI's `hotline serve` child processes (serve.rs free_address/hotline(), pair.rs reservations, relay.rs:323) still reserve a loopback port and drop it, because a bound socket cannot be passed to a child process without inheriting the fd. The served_listener test that waits for an occupied socket keeps its real bind, since that wait is what it tests. tests/processes.rs:387 and tests/voice.rs:2818 (T11, outside this row) are untouched. make check, make verify and make precheck pass.
