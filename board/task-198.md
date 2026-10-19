---
id: task-198
title: "hotline-tools: the shell and workspace tools as their own crate"
column: todo
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
updatedAt: "2026-10-19T06:10:21.252Z"
---

## Description
tools reaches driver only for CapabilityLease and CapabilityEpoch (driver.rs:46-130, used at tools/workspace.rs:3 and tools/workspace/tests.rs), and process_windows (in hotline-support by now). Move those two types (and only what they need) into a capability module the tools crate owns — not hotline-contract: they are runtime authority, not wire types, and carry no ts-rs derive — then move tools to crates/hotline-tools. They are pub(crate) today and must become pub across the crate boundary: keep every field private so a lease cannot be forged or revived from outside, and say in the commit/PR, per docs/security.md, that the default, the enforcement points and revocation are unchanged and what widened (the residual risk).

This is the workspace isolation boundary: docs/security.md's citations follow it (python3 scripts/security-citations passes), workspace-isolation.yml's path filter names crates/hotline-tools/** and its ignored-test step names the new crate's test paths, and the bubblewrap tests still run there with --ignored. Run the isolation tests on this Mac before landing: the ignored shell tests in hotline-tools and hotline-core's desk test workspace_reach_keeps_another_projects_env_out_of_the_tools.

Lead's note: this runs after task-197 because both would add to the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md, AGENTS.md); in parallel they would not land cleanly.
