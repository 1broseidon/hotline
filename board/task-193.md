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
updatedAt: "2026-10-19T06:06:58.241Z"
---

## Description
Measure before moving anything, on this Mac (Apple silicon), each run twice and the second kept:
- cold: `cargo clean -p hotline-core && cargo build -p hotline-core --timings` (keep the summary, not the HTML), and the same for `cargo test -p hotline-core --no-run`;
- incremental: touch one file in voice/, one in store.rs, one in contract/, and time `cargo build -p hotline-core` and `cargo test -p hotline-core --no-run` after each;
- note how long hotline-core's own compile takes against its dependencies in the timings.
Write docs/adr/0007-split-hotline-core.md (next free number; check docs/adr): the timings, the module dependency map (crate::X references between top-level modules; script it, and put the script in scripts/ so later stages re-run it), the leaves, the one-link cuts, the strongly connected core, and the target crate graph for this epic's stages. No code changes.
