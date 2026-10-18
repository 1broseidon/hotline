---
id: task-115
title: "P2-3 (A): Patch ACP ZIP extraction (F1)"
column: in-progress
position: 4
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:43.518Z"
relatedFiles:
  - crates/hotline-core/Cargo.toml
  - Cargo.lock
  - crates/hotline-core/src/driver/acp/install.rs
  - docs/security.md
updatedAt: "2026-10-18T00:32:59.528Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:32:59.528Z"
    reworkCount: 0
dependsOn:
  - task-113
  - task-114
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:36.022Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:32:05.196Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 3 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch ACP ZIP extraction (F1).** This is a capability change; see below

- Source: dead-code F1, task 2; also lints `case_sensitive_file_extension_comparisons` at `install.rs:112` and dead-code F8 (`zip` features)
- Files: `crates/hotline-core/Cargo.toml:42`, `Cargo.lock`, `crates/hotline-core/src/driver/acp/install.rs` and its tests, `docs/security.md` (new row)
- Collides with: 2, 4, 5 (lockfile); 11 (Cargo.toml)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:50:40.303Z: [claude] Done on dispatch/task-115 (21ca5528). zip 4 -> 9.0.5 for hotline-core (features: deflate-flate2 only, so zopfli is out of core; F8). install.rs: unpack picks the format by Path extension, case-insensitive (lint fixed, test an_archive_is_known_by_its_extension_in_any_case). New unzip/untar check every entry through one admit(): links (symlink or hard link), paths outside the folder, set-id and world-writable modes are refused, and the error drops the scratch TempDir. Zip entries are checked from the central directory before extract; tar entries one at a time before unpack_in. zip 9 already clears set-id bits when it reads a mode, so a set-id zip installs with the bits cleared (tested); world-writable zip and set-id tar are refused. 4 new denial tests; I checked each rule by disabling it and the matching test fails. docs/security.md: new section 'An agent installed from the ACP registry' (default, grant, enforcement, tests, revocability, residual risk) + a regression-matrix row under the ACP callbacks row. cargo-deny: RUSTSEC-2026-0367..0369 now only on zip 4.6.1 (tauri-plugin-updater, task 4) and 2.4.2 (sherpa build, task 5); zip 4.6.1 stays in Cargo.lock for the updater. Residual: an archive that needs a link inside itself (e.g. a node_modules/.bin symlink) will no longer install; no registry agent I could check needs one, but a failure like that would show up as 'the archive's entry X is a link'. Tests stay in-file (install.rs tests now outgrow the code; the acp/ folder uses auth_tests.rs, so the split belongs with structure tasks). make check and make precheck (7/7) pass; one make check run hit an unrelated flake, relay_tests::deselecting_the_relay... with an 'Address already in use' error, which passed on rerun. Nothing left unwired.
