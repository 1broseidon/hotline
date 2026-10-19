---
id: task-195
title: "hotline-websearch: web search as its own crate"
column: todo
position: 3
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/websearch.rs
  - crates/hotline-core/src/websearch
parentId: epic-16
createdAt: "2026-10-19T06:06:59.331Z"
dependsOn:
  - task-194
  - task-198
updatedAt: "2026-10-19T06:11:05.054Z"
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-websearch/Cargo.toml
      description: Web search on hotline-contract, with its own HTTP and provider dependencies
  validation:
    commands:
      - test -f crates/hotline-websearch/src/lib.rs
      - test ! -e crates/hotline-core/src/websearch.rs && test ! -e crates/hotline-core/src/websearch
      - "! grep -qE '^hotline-(core|store|tools)' crates/hotline-websearch/Cargo.toml"
      - grep -A1 '^\[lints\]' crates/hotline-websearch/Cargo.toml | grep -q 'workspace = true'
      - grep -q hotline-websearch docs/standards.md && grep -q hotline-websearch AGENTS.md
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Moves only: no behaviour change"
    - Dependencies nothing else uses leave hotline-core
    - contract.ts byte-identical
  metrics:
    readyAt: "2026-10-19T06:11:05.054Z"
---

## Description
websearch (about 6,200 lines) reaches only contract. Move it to crates/hotline-websearch depending on hotline-contract; its HTTP and provider dependencies move with it and leave hotline-core's Cargo.toml if nothing else uses them (cargo machete agrees, and make check runs it).

Lead's note: this runs last of the moves, after task-198, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md); it is the smallest and lowest-risk, so it waits rather than the store and tools moves.
