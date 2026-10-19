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
updatedAt: "2026-10-19T06:07:12.791Z"
---

## Description
tools reaches driver only for CapabilityLease and CapabilityEpoch. Move those two types (and only what they need) down to hotline-contract or a capability module the tools crate owns, then move tools to crates/hotline-tools. This is the workspace isolation boundary: docs/security.md's citations follow it, workspace-isolation.yml's path filter names the new crate's paths, and the bubblewrap tests still run there with --ignored. Run the isolation tests on this Mac before landing.
