---
id: task-176
title: "Phase 2 holds together: precheck-full, verify, and every gate in make check"
column: todo
position: 65
description: |-
  The epic's closing check, last of all, on release/0.40.0 with every other child of epic-13 landed. The epic is done when every child has landed, `make precheck-full` passes, and the gates in F are in `make check`.

  1. `make precheck-full` and `make verify` pass. Fix what fails, in the smallest commits, each stating its invariant.
  2. `make check` runs the F gates: the `[workspace.lints]` table (every crate has `[lints] workspace = true`), Biome lint in `make ui-check`, knip and the contract preflight, cargo-machete, cargo-deny advisories and licenses, and `bun audit`. Each is in the Makefile and passes.
  3. No `mod.rs` under any `crates/*/src` (only `tests/common/mod.rs`), no `#[path]`, no `_tests.rs` file; no production `.rs` over 1,500 lines without a note in the module map saying why.
  4. `docs/security.md` cites only tests that exist: every test name it cites is found by `git grep`. Write a short script for this if none exists and leave it in `scripts/`, wired into `make check`.
  5. `docs/development.md`'s module map matches the tree; `docs/standards.md` exists.

  Write what you checked, and anything left over for George, as a note on epic-13.
priority: high
tags:
  - phase2
  - integration
relatedFiles:
  - Makefile
  - scripts/precheck
  - docs/security.md
  - docs/development.md
  - docs/standards.md
  - Cargo.toml
  - deny.toml
parentId: epic-13
createdAt: "2026-10-17T22:45:36.945Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck-full
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
updatedAt: "2026-10-17T22:48:38.570Z"
dependsOn:
  - task-161
  - task-162
  - task-163
  - task-166
  - task-167
  - task-171
---

## Description
The epic's closing check, last of all, on release/0.40.0 with every other child of epic-13 landed. The epic is done when every child has landed, `make precheck-full` passes, and the gates in F are in `make check`.

1. `make precheck-full` and `make verify` pass. Fix what fails, in the smallest commits, each stating its invariant.
2. `make check` runs the F gates: the `[workspace.lints]` table (every crate has `[lints] workspace = true`), Biome lint in `make ui-check`, knip and the contract preflight, cargo-machete, cargo-deny advisories and licenses, and `bun audit`. Each is in the Makefile and passes.
3. No `mod.rs` under any `crates/*/src` (only `tests/common/mod.rs`), no `#[path]`, no `_tests.rs` file; no production `.rs` over 1,500 lines without a note in the module map saying why.
4. `docs/security.md` cites only tests that exist: every test name it cites is found by `git grep`. Write a short script for this if none exists and leave it in `scripts/`, wired into `make check`.
5. `docs/development.md`'s module map matches the tree; `docs/standards.md` exists.

Write what you checked, and anything left over for George, as a note on epic-13.
