---
id: task-194
title: "hotline-contract: the wire and tape types as their own crate"
column: in-progress
position: 2
priority: high
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/contract.rs
  - crates/hotline-core/src/contract
  - ui/src/generated/contract.ts
parentId: epic-16
createdAt: "2026-10-19T06:06:58.782Z"
dependsOn:
  - task-193
updatedAt: "2026-10-19T07:41:48.097Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-contract/Cargo.toml
      description: The contract crate, on no other hotline crate
    - type: file
      path: crates/hotline-contract/src/lib.rs
      description: The wire and tape types and their ts-rs export
  validation:
    commands:
      - test -f crates/hotline-contract/src/lib.rs
      - test ! -e crates/hotline-core/src/contract.rs && test ! -e crates/hotline-core/src/contract
      - "! grep -qE '^hotline-' crates/hotline-contract/Cargo.toml"
      - grep -A1 '^\[lints\]' crates/hotline-contract/Cargo.toml | grep -q 'workspace = true'
      - grep -q hotline-contract docs/standards.md && grep -q hotline-contract AGENTS.md
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Moves only: no behaviour change, no renamed wire field"
    - ui/src/generated/contract.ts byte-identical from an unfiltered cargo test
    - Tape event shapes unchanged
  metrics:
    readyAt: "2026-10-19T06:11:03.996Z"
    pickedUpAt: "2026-10-19T07:41:48.097Z"
    reworkCount: 1
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T06:38:57.883Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T07:41:39.477Z"
---

## Description
Move contract.rs and contract/ into crates/hotline-contract. It depends on no other hotline crate (replace the doc link to crate::paths::thread_key with words). hotline-core depends on it and keeps `pub use hotline_contract as contract` (or moves callers; your choice, say which in the commit). The ts-rs export moves with the types and still writes ui/src/generated/contract.ts byte-identical; check the generation test and make check's contract sync still run it, and that hotline-mobile's sync:contract source path (../hotline/ui/src/generated) is unchanged. hotline-app and hotline-cli build.

## Log
- 2026-10-19T07:41:38.946Z: Landing sent back (1 of 1): with release/0.41.0 brought in, `make check` failed: stopped after 15 minutes. Fix it on this branch, commit, and run the checks again.
- 2026-10-19T07:48:06.797Z: [claude] Re-landing: release/0.41.0 (bdce54fc) is already in dispatch/task-194 and has not moved, so there was nothing to merge and no code to fix; the branch is unchanged at 37cb44f4. Here, make check passes in 4m11s and make precheck in 93s (all 8 legs); contract.ts is byte-identical after both, and all of the contract's validation commands pass. Unlike earlier runs, nothing hung in this one, so the 15-minute stop looks like a cold target/ in the landing checkout (clippy and test both build from cold), not a hang on this branch. If it recurs, warm target/ or raise the limit. The slowest non-cargo step is scripts/test_githooks.py at 57s: its tsc runs through the mise node shim in a temp dir, which took about 50s per call. Same for the tasks after this one: hotline-core keeps `pub use hotline_contract as contract`, so no caller moved; hotline-contract depends on no hotline crate.
