---
id: task-103
title: "Audit: Rust structure and module layout, with a target tree and file splits"
column: review
position: 1
priority: high
tags:
  - audit
  - structure
parentId: epic-12
createdAt: "2026-10-17T19:28:03.176Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-17T19:30:17.898Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T19:44:50.921Z"
    duration: 873
updatedAt: "2026-10-17T19:44:50.921Z"
relatedFiles:
  - docs/audits/0.40/structure.md
  - crates/hotline-core/src
  - crates/hotline-app/src
  - crates/hotline-cli/src
  - Cargo.toml
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T19:29:48.141Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T19:29:49.832Z"
assignee: claude
---

## Description
Audit: Rust structure and module layout.

- Map hotline-core's modules: size, cohesion, who depends on whom (cargo modules or a hand-made graph).
- Propose a target tree. Pick one module convention (Rust 2018 foo.rs + foo/ is the likely choice) and one test layout (sibling tests.rs vs tests/ dir vs in-file mod tests; integration tests in crates/*/tests).
- Propose a file-size budget, and splits for each file over it: acp.rs, session/mod.rs, contract.rs, voice/mod.rs, sides.rs, rig.rs, wire/mod.rs, turn.rs, workspace.rs, and the 7k test files. For each split, name the new files and what moves.
- Decide whether any boundary deserves its own crate (contract/wire types, remote, voice, tools), weighing build-time gains against churn.
- Check the app and cli crates for logic that belongs in core.
- Report: docs/audits/0.40/structure.md.

## Log
- 2026-10-17T19:44:50.593Z: [claude] For the tasks after this: the report is at docs/audits/0.40/structure.md and is committed on branch dispatch/task-103. Its 21 Phase 2 tasks run in order. Tasks 1–2 rename the 16 mod.rs files to foo.rs and give test files one naming style; do them first, so every later split lands on the new paths. Task 3 makes contract a leaf before task 4 splits it, and both must leave ui/src/generated/contract.ts byte-identical after an unfiltered cargo test. session/tests.rs and wire/tests.rs are the merge hot spots (76 and 81 commits since 2026-09-01), so split them early. No new crates: incremental check is 3–6s and the test rebuild 12s; revisit if the test rebuild passes 30s or the contract gets a second consumer. Building needs .speech-engine/ copied in from the main checkout.
