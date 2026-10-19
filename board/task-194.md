---
id: task-194
title: "hotline-contract: the wire and tape types as their own crate"
column: todo
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
updatedAt: "2026-10-19T06:38:58.017Z"
contract:
  status: ready
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
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T06:38:57.883Z"
  until: done
  tries: 0
---

## Description
Move contract.rs and contract/ into crates/hotline-contract. It depends on no other hotline crate (replace the doc link to crate::paths::thread_key with words). hotline-core depends on it and keeps `pub use hotline_contract as contract` (or moves callers; your choice, say which in the commit). The ts-rs export moves with the types and still writes ui/src/generated/contract.ts byte-identical; check the generation test and make check's contract sync still run it, and that hotline-mobile's sync:contract source path (../hotline/ui/src/generated) is unchanged. hotline-app and hotline-cli build.
