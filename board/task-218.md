---
id: task-218
title: "Slice 4: pair threads on the one builder and loop"
column: in-progress
position: 14
priority: medium
tags:
  - threads
assignee: codex
relatedFiles:
  - crates/hotline-core/src/session
parentId: epic-20
createdAt: "2026-10-19T19:59:31.591Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-core/src/session/peers/sessions.rs
      description: peer_session builds through Room::thread_agent
  validation:
    commands:
      - make check
      - "! git grep -n 'runner::drive(' -- crates/hotline-core/src/session/peers"
      - cargo test -p hotline-core --test threads
      - cargo test -p hotline-core --test desk
  constraints:
    - Peer exchange harness tests pass without changing their assertions
    - "Do not touch session.rs's call_posted, prompt.rs's pending_calls, dm.rs's escalation or session/files.rs: task-216 runs alongside"
    - Run cargo test -p hotline-core unfiltered before committing ui/src/generated/contract.ts (AGENTS.md rule 3)
  metrics:
    readyAt: "2026-10-19T20:03:13.573Z"
    pickedUpAt: "2026-10-19T20:03:55.078Z"
    reworkCount: 0
updatedAt: "2026-10-19T20:03:55.078Z"
dispatch:
  agent: codex
  model: gpt-6.1-sol
  effort: high
  by: lead
  at: "2026-10-19T20:03:30.245Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:03:36.062Z"
---

## Description
ADR 0008 §6 slice 4: peer_session builds through Room::thread_agent and drives through Threads::drive. Removes peer_session's own builder, the pair's use of runner::drive and the stale comment in Policy::of. Peer exchange harness tests pass unchanged.

## Log
- 2026-10-19T20:04:29.604Z: [codex] Picked up contract. Scope from cymbal impact: peer_session is called only by peer delivery; shared builder also serves DM/work/run. Preserve dual leases, directional history and linked caller identity; replace pair driving with a Witness on Threads::drive and remove the redundant startup guard. Reserved task-216 paths stay untouched.
