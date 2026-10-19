---
id: task-218
title: "Slice 4: pair threads on the one builder and loop"
column: review
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
  status: done
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
    deliveredAt: "2026-10-19T20:08:25.244Z"
    duration: 270
updatedAt: "2026-10-19T20:13:20.988Z"
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
    ended: "2026-10-19T20:13:20.988Z"
    outcome: review
---

## Description
ADR 0008 §6 slice 4: peer_session builds through Room::thread_agent and drives through Threads::drive. Removes peer_session's own builder, the pair's use of runner::drive and the stale comment in Policy::of. Peer exchange harness tests pass unchanged.

## Log
- 2026-10-19T20:04:29.604Z: [codex] Picked up contract. Scope from cymbal impact: peer_session is called only by peer delivery; shared builder also serves DM/work/run. Preserve dual leases, directional history and linked caller identity; replace pair driving with a Witness on Threads::drive and remove the redundant startup guard. Reserved task-216 paths stay untouched.
- 2026-10-19T20:06:29.680Z: [codex] Implementation uses the shared Opening/ThreadAgent lifecycle and a pair Witness on Threads::drive. Removed StartingPeer plus unused runner::drive/drive_with/Calls; the written-version test now exercises drive_updates directly with unchanged assertions. Added builder coverage for both history orientations on Hotline and ACP, with home checkpoints stripped. Full check, threads/desk harnesses and precheck are running.
- 2026-10-19T20:12:33.340Z: [codex] Handoff for later epic-20 slices: Opening is unchanged. Pairs pass target as persona, caller as opener, and the dual target/caller lease. The builder resolves linked openers through participant(), uses peers::said_in to orient pair history, and selects peer_preamble/Tools::Peer; pair bindings are not persisted. PairTurn implements Witness only for receipts and Waiting markers; first contact, exchange serialization/cap and permission answers remain in peers. Calls can reuse the same Opening/Threads::drive path next. Pair startup now inherits the shared builder's Computer::Lease fallback (continue without an unavailable computer) and refusal of a Hotline Agent arm with no provider.
- 2026-10-19T20:13:05.166Z: [codex] Finished on dispatch/task-218 in commit 207e172f (Pair threads share the agent builder and turn loop.). peer_session now builds through Room::thread_agent and pair exchanges use Threads::drive; redundant builder/startup guard/runner wrappers and stale policy comment are removed. Existing peer harness assertions are unchanged. Contract delivered and validated in this worktree: make check, no peers runner::drive calls, threads and desk harnesses all pass. make precheck passes all 8 legs, including Windows and isolated UI tests. Working tree is clean; ready for human review and merge.
