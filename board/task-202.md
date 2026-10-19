---
id: task-202
title: "Core + contract: the team lead role, set, moved and kept"
column: todo
position: 2
priority: high
tags:
  - core
  - contract
assignee: claude
relatedFiles:
  - crates/hotline-contract/src/persona.rs
  - crates/hotline-contract/src/command.rs
  - crates/hotline-contract/src/replies.rs
  - crates/hotline-room/src/room.rs
  - crates/hotline-core/src/wire/commands/persona.rs
  - crates/hotline-core/src/wire/roster.rs
  - crates/hotline-core/src/wire/seat.rs
  - crates/hotline-core/src/wire/tests/roster.rs
  - crates/hotline-core/src/wire/tests/seats.rs
  - crates/hotline-core/tests/desk/teammates.rs
  - docs/wire.md
  - docs/design.md
parentId: epic-17
createdAt: "2026-10-19T13:59:37.690Z"
updatedAt: "2026-10-19T20:02:51.811Z"
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-contract/src/persona.rs
      description: the lead on the roster and its commands in the contract
    - type: test
      path: crates/hotline-core/tests/desk/teammates.rs
      description: "harness tests named for the lead: auto-set, move, clear, delete-to-one, upgrade"
    - type: docs
      path: docs/wire.md
      description: the lead commands and roster field
  validation:
    commands:
      - cargo test -p hotline-core --test desk -- --list 2>/dev/null | grep -q lead
      - grep -qi lead docs/wire.md
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make verify
  constraints:
    - hotline-core never depends on Tauri
    - "One writer per stream: only the core appends to the room stream"
    - "Tapes are untouched: no change to tape event shapes or segment layout"
    - "No first-contact grant in this task: it belongs to task-203 (0.42)"
    - Never kill by name, path or port; use HOTLINE_DATA_DIR for any manual run
  metrics:
    readyAt: "2026-10-19T20:02:26.422Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:02:49.610Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:02:51.811Z"
---

## Description
The room holds one lead (a persona id) in the room stream and sends it on the roster; a client that predates it ignores it. Commands to make a teammate lead and to clear it, under the owner checks other room commands use (persona.pin and persona.delete are the precedent; a seat that may not change the roster is refused). Rules, all in the core: a desk with exactly one teammate and no lead sets that one, at create and at load (so onboarding's first teammate becomes lead, and an upgraded one-teammate desk gets its lead); deleting the lead clears it, and if exactly one teammate is left it becomes lead by the same rule; nothing ever picks among two or more. Making another teammate lead moves the role (one per desk). The lead keeps its own model, memory and tools: being lead changes nothing else about the persona in 0.41.

Out of scope, moved to 0.42 (epic-21, task-203): the lead's grant to hand work to teammates on a desk call without first-contact approval. Desk calls do not reach the lead until task-203, so a grant here would have no caller.

Proof: wire tests beside the code (roster carries the lead; set, move, clear; refused for a seat that cannot change the roster) and the desk harness (crates/hotline-core/tests/desk/teammates.rs) driving the real core over the wire: first teammate becomes lead; a second does not change it; set moves it; delete of the lead with two left leaves none; delete down to one sets that one; a room stream written before the field (one teammate, no lead) loads with that teammate as lead. Name the harness tests with "lead" in them. docs/wire.md names the commands and the roster field; docs/design.md records the role and its rules in a short paragraph. Run cargo test -p hotline-contract unfiltered so ui/src/generated/contract.ts is written whole, and commit it.
