---
id: task-193
title: "Baseline: build timings and the module map, in an ADR"
column: todo
position: 1
priority: high
tags:
  - crate-split
  - measure
assignee: claude
relatedFiles:
  - Cargo.toml
  - docs/adr
parentId: epic-16
createdAt: "2026-10-19T06:06:19.521Z"
updatedAt: "2026-10-19T06:11:20.001Z"
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: Before timings, module map, leaves, cuts, core cycle, target crate graph
    - type: file
      path: scripts/module-map
      description: Re-runnable script printing crate::X references between top-level modules
  validation:
    commands:
      - test -f docs/adr/0007-split-hotline-core.md
      - grep -q '^## Before' docs/adr/0007-split-hotline-core.md
      - grep -q 'cargo build -p hotline-core' docs/adr/0007-split-hotline-core.md
      - test -x scripts/module-map && scripts/module-map | grep -q websearch
      - git diff --quiet release/0.41.0...HEAD -- crates Cargo.toml Cargo.lock ui
      - python3 -m unittest discover -s scripts -p 'test_*.py'
  constraints:
    - No code changes under crates/, Cargo.toml, Cargo.lock or ui/
    - Each timing run twice, the second kept, on this Mac
  metrics:
    readyAt: "2026-10-19T06:11:03.728Z"
dispatch:
  agent: claude
  model: sonnet
  effort: high
  by: lead
  at: "2026-10-19T06:11:18.158Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T06:11:20.001Z"
---

## Description
Measure before moving anything, on this Mac (Apple silicon), each run twice and the second kept:
- cold: `cargo clean -p hotline-core && cargo build -p hotline-core --timings` (keep the summary, not the HTML), and the same for `cargo test -p hotline-core --no-run`;
- incremental: touch one file in voice/, one in store.rs, one in contract/, and time `cargo build -p hotline-core` and `cargo test -p hotline-core --no-run` after each;
- note how long hotline-core's own compile takes against its dependencies in the timings.
Write docs/adr/0007-split-hotline-core.md (0007 is the next free number on release/0.41.0): the exact commands, the timings under a heading `## Before` (task-199 adds `## After` beside it with the same commands, so write them so they can be re-run verbatim), the module dependency map (crate::X references between top-level modules, brace imports included; script it as an executable `scripts/module-map`, run from the repo root with no arguments, so later stages re-run it), the leaves, the one-link cuts, the strongly connected core, and the target crate graph for this epic's stages. Settle the name of the leaf-utilities crate there (hotline-support unless it reads wrong) and say it in your final note, so the lead can fix task-196's contract if it changes. No code changes: validation refuses any diff under crates/, Cargo.toml, Cargo.lock or ui/.
