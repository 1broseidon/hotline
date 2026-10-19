---
id: task-198
title: "hotline-tools: the shell and workspace tools as their own crate"
column: review
position: 6
priority: medium
tags:
  - crate-split
  - security
assignee: claude
relatedFiles:
  - crates/hotline-core/src/tools.rs
  - crates/hotline-core/src/tools
  - docs/security.md
  - .github/workflows/workspace-isolation.yml
parentId: epic-16
createdAt: "2026-10-19T06:07:00.939Z"
dependsOn:
  - task-194
  - task-196
  - task-197
updatedAt: "2026-10-19T08:30:50.882Z"
contract:
  status: delivered
  deliverables:
    - type: file
      path: crates/hotline-tools/Cargo.toml
      description: The shell and workspace tools with the capability lease they check
    - type: file
      path: .github/workflows/workspace-isolation.yml
      description: Path filter and ignored-test step on the new crate
  validation:
    commands:
      - test -f crates/hotline-tools/src/lib.rs
      - test ! -e crates/hotline-core/src/tools.rs && test ! -e crates/hotline-core/src/tools
      - "! grep -qE '^hotline-(core|store|websearch)' crates/hotline-tools/Cargo.toml"
      - grep -A1 '^\[lints\]' crates/hotline-tools/Cargo.toml | grep -q 'workspace = true'
      - grep -q 'crates/hotline-tools/' .github/workflows/workspace-isolation.yml
      - "! grep -q 'crates/hotline-core/src/tools' .github/workflows/workspace-isolation.yml docs/security.md"
      - python3 scripts/security-citations
      - grep -q hotline-tools docs/standards.md && grep -q hotline-tools AGENTS.md
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - cargo test -p hotline-tools --locked -- --ignored shell
      - cargo test -p hotline-core --locked --test desk -- --ignored workspace_reach_keeps_another_projects_env_out_of_the_tools
      - make precheck
  constraints:
    - "Moves only: no behaviour change"
    - Lease and epoch fields stay private; revocation unchanged
    - PR states default, grant source, enforcement points, tests, residual risk per docs/security.md
    - contract.ts byte-identical
  metrics:
    readyAt: "2026-10-19T06:11:04.784Z"
    pickedUpAt: "2026-10-19T08:18:11.932Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T08:30:50.882Z"
    duration: 759
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T08:16:32.391Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T08:16:34.483Z"
---

## Description
tools reaches driver only for CapabilityLease and CapabilityEpoch (driver.rs:46-130, used at tools/workspace.rs:3 and tools/workspace/tests.rs), and process_windows (in hotline-support by now). Move those two types (and only what they need) into a capability module the tools crate owns — not hotline-contract: they are runtime authority, not wire types, and carry no ts-rs derive — then move tools to crates/hotline-tools. They are pub(crate) today and must become pub across the crate boundary: keep every field private so a lease cannot be forged or revived from outside, and say in the commit/PR, per docs/security.md, that the default, the enforcement points and revocation are unchanged and what widened (the residual risk).

This is the workspace isolation boundary: docs/security.md's citations follow it (python3 scripts/security-citations passes), workspace-isolation.yml's path filter names crates/hotline-tools/** and its ignored-test step names the new crate's test paths, and the bubblewrap tests still run there with --ignored. Run the isolation tests on this Mac before landing: the ignored shell tests in hotline-tools and hotline-core's desk test workspace_reach_keeps_another_projects_env_out_of_the_tools.

Lead's note: this runs after task-197 because both would add to the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md, AGENTS.md); in parallel they would not land cleanly.

From task-194, task-196 and task-197 (all landed on release/0.41.0): hotline-core re-exports each moved crate under its old module name (crate::contract, crate::paths, crate::store, crate::log, crate::sent, …) so no caller moved; do the same for tools (and re-export the capability types from driver under their old names) unless moving callers is plainly simpler, and say which in the commit. task-197 also had to teach scripts/module-map and scripts/security-citations about the new crate's paths and added a line to docs/adr/0007-split-hotline-core.md and docs/development.md: follow the same pattern for hotline-tools. Commit Cargo.lock with the move. Run `make check` and `make precheck` in the foreground (one call each, long timeout) and end your turn only once both have passed and the task is in Review; ending while they run in the background loses the hand-off.

## Log
- 2026-10-19T08:30:57.081Z: [claude] On dispatch/task-198 (c3d6eef4). crates/hotline-tools holds the workspace and shell tools plus a `capability` module with CapabilityLease/CapabilityEpoch and their 7 tests, moved out of driver.rs. Its only Hotline deps are hotline-contract and hotline-support (process_windows). Callers stayed put: core has `pub use hotline_tools as tools`, and driver has `pub(crate) use hotline_tools::capability::{CapabilityEpoch, CapabilityLease}`. Security: the default, grant source, enforcement points and revocation are unchanged. What widened (the residual risk, in the commit): the lease types and their methods, the shell types (CommandOutcome/State, OutputTail, RunCommandArgs, STDERR_LINE) and the Workspace methods core calls went from pub(crate) to pub, so app and cli can reach them via hotline_core::tools. Every field stays private, so no crate can forge or revive a lease. cap-std, ignore, grep-*, globset moved from core to tools. scripts/module-map: ("tools","driver") was the last planned cut, so I deleted PLANNED_CUTS and print_after_cuts. security-citations gained a `crates` root, and security.md now cites `hotline-tools/src/...`. workspace-isolation.yml and scripts/precheck run `cargo test -p hotline-tools -- --ignored shell` and the desk test via `--test desk`. Docs updated: standards, development layout, AGENTS.md, ADR 0007 cut 3, sessions.md, security.md grant lifecycle. On this Mac, the shell --ignored tests (2) and workspace_reach_keeps_another_projects_env_out_of_the_tools pass. The desk test is not #[ignore] on macOS, so with --ignored it filters to 0 and passes; I ran it plainly. make check passes (3m13s), make precheck passes (98s, all 8 legs, Windows included), and contract.ts is byte-identical. For task-199: nothing is left unwired; the core no longer depends on cap-std or ignore directly.
