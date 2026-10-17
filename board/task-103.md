---
id: task-103
title: "Audit: Rust structure and module layout, with a target tree and file splits"
column: todo
position: 1
priority: high
tags:
  - audit
  - structure
parentId: epic-12
createdAt: "2026-10-17T19:28:03.176Z"
---

## Description
Audit: Rust structure and module layout.

- Map hotline-core's modules: size, cohesion, who depends on whom (cargo modules or a hand-made graph).
- Propose a target tree. Pick one module convention (Rust 2018 foo.rs + foo/ is the likely choice) and one test layout (sibling tests.rs vs tests/ dir vs in-file mod tests; integration tests in crates/*/tests).
- Propose a file-size budget, and splits for each file over it: acp.rs, session/mod.rs, contract.rs, voice/mod.rs, sides.rs, rig.rs, wire/mod.rs, turn.rs, workspace.rs, and the 7k test files. For each split, name the new files and what moves.
- Decide whether any boundary deserves its own crate (contract/wire types, remote, voice, tools), weighing build-time gains against churn.
- Check the app and cli crates for logic that belongs in core.
- Report: docs/audits/0.40/structure.md.
