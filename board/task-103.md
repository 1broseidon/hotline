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
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/audits/0.40/structure.md
      description: Structure audit report
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/structure.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/structure.md && grep -q '^## Leave alone' docs/audits/0.40/structure.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/structure.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.rs:[0-9]+' docs/audits/0.40/structure.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/structure.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/structure.md; add no tools, config or code to the repo"
    - "Report has exactly these sections: '## Findings' (ranked severity x effort, each with file:line evidence), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - If a cargo run rewrites ui/src/generated/contract.ts, restore it before committing
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
updatedAt: "2026-10-17T19:29:18.236Z"
---

## Description
Audit: Rust structure and module layout.

- Map hotline-core's modules: size, cohesion, who depends on whom (cargo modules or a hand-made graph).
- Propose a target tree. Pick one module convention (Rust 2018 foo.rs + foo/ is the likely choice) and one test layout (sibling tests.rs vs tests/ dir vs in-file mod tests; integration tests in crates/*/tests).
- Propose a file-size budget, and splits for each file over it: acp.rs, session/mod.rs, contract.rs, voice/mod.rs, sides.rs, rig.rs, wire/mod.rs, turn.rs, workspace.rs, and the 7k test files. For each split, name the new files and what moves.
- Decide whether any boundary deserves its own crate (contract/wire types, remote, voice, tools), weighing build-time gains against churn.
- Check the app and cli crates for logic that belongs in core.
- Report: docs/audits/0.40/structure.md.
