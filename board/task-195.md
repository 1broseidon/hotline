---
id: task-195
title: "hotline-websearch: web search as its own crate"
column: review
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
updatedAt: "2026-10-19T08:44:25.148Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T08:33:27.261Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T08:44:25.148Z"
    duration: 658
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T08:33:11.064Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T08:33:12.898Z"
---

## Description
websearch (about 6,200 lines) reaches only contract. Move it to crates/hotline-websearch depending on hotline-contract; its HTTP and provider dependencies move with it and leave hotline-core's Cargo.toml if nothing else uses them (cargo machete agrees, and make check runs it).

Lead's note: this runs last of the moves, after task-198, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md); it is the smallest and lowest-risk, so it waits rather than the store and tools moves.

From task-194, task-196, task-197 and task-198 (all landed on release/0.41.0): hotline-core re-exports each moved crate under its old module name (crate::contract, crate::paths, crate::store, crate::tools, …) so no caller moved; do the same for websearch (`pub use hotline_websearch as websearch;` or equivalent) unless moving callers is plainly simpler, and say which in the commit. The earlier moves taught scripts/module-map and scripts/security-citations about each new crate's paths and added a line to docs/adr/0007-split-hotline-core.md and docs/development.md: follow the same pattern for hotline-websearch (if docs/security.md cites websearch code, its citations must follow; python3 scripts/security-citations passes). The new crate gets `[lints] workspace = true`, a license, and a crate-level doc saying what it owns and that it must never depend on hotline-core, hotline-store or hotline-tools. Commit Cargo.lock with the move. Run `make check` and `make precheck` in the foreground (one call each, long timeout) and end your turn only once both have passed and the task is in Review; ending while they run in the background loses the hand-off.

## Log
- 2026-10-19T08:44:28.455Z: [claude] On dispatch/task-195 (0019260a). crates/hotline-websearch holds web search, depending on hotline-contract alone. Callers did not move: core has `pub(crate) use hotline_websearch as websearch`. One thing crossed back: core's mcp/server tests used websearch's test-only mock providers, so that mock moved out of tests.rs into `mock.rs`. It is public only under a `test-support` feature (optional axum), and core turns it on as a dev-dependency. whatlang left core; reqwest, regex, url, async-trait and futures-util stay because core uses them elsewhere (machete is clean). security.md cites no websearch code and module-map had no cut for it, so neither script changed. Docs updated: standards, AGENTS.md, development layout, and an ADR 0007 note. make check passes (3m52s), and so does make precheck (all 8 legs, 98s); contract.ts is byte-identical. Nothing is left unwired. This was the last crate move, so task-199 (the timings again) is next.
