---
id: task-167
title: "P2-52 (F): The `[workspace.lints]` table in one change"
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:10.558Z"
relatedFiles:
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-app/Cargo.toml
  - crates/hotline-cli/Cargo.toml
updatedAt: "2026-10-18T11:19:50.768Z"
contract:
  status: delivered
  validation:
    commands:
      - grep -q '^\[workspace.lints' Cargo.toml
      - grep -q 'rust-version' Cargo.toml
      - for c in core app cli; do grep -q '^\[lints\]' crates/hotline-$c/Cargo.toml || exit 1; done
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T10:49:39.580Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T11:17:43.064Z"
    duration: 1683
dependsOn:
  - task-160
  - task-156
  - task-175
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:27.779Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T10:49:32.710Z"
    ended: "2026-10-18T11:19:50.768Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T11:19:50.379Z"
---

## Description
Phase 2 group F (Gates), plan row 52 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The `[workspace.lints]` table in one change.** It holds every lint that 39–42 made clean, the zero-hit denies, and `allow_attributes_without_reason`. Add `rust-version`, and `[lints] workspace = true` in all three crates. `-D warnings` stays in the Makefile

- Source: lints, tasks 1 and 4 (C4, C5)
- Files: `Cargo.toml`, three crate manifests
- Collides with: 13
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T11:17:42.591Z: [claude] Done on dispatch/task-167 (9686b80f, 68124de7, a2c0d93f). Cargo.toml has [workspace.lints.rust] and [workspace.lints.clippy], and the three crates declare `[lints] workspace = true` and `rust-version.workspace = true`. rust-version is 1.95: I built on 1.91 (the newest deps' MSRV) and on 1.94, and both fail on `if let` guards. The table holds: the zero-hit denies (dbg_macro, todo, unimplemented, get_unwrap, rc_buffer, try_err, empty_drop, lossy_float_literal, verbose_file_reads, unsafe_op_in_unsafe_fn), the zero-hit rust warns (keyword_idents at priority -1 because it is a group, single_use_lifetimes, trivial_numeric_casts, unused_import_braces, unused_lifetimes), allow_attributes_without_reason, and the 52 clippy lints that rows 39 and 40 cleaned. LEFT OFF because they still fire: trivial_casts (row 39's `[] as [T; 0]` asserts are trivial casts), needless_collect, or_fun_call, unnecessary_wraps and needless_continue (neither row covered them), allow_attributes (C5), string_to_string (renamed and removed on 1.99), and row 40's deliberate exclusions. Fixed to make the gate hold: four allows had no reason (records.rs now uses cfg_attr(not(test), expect), push.rs and mcp.rs use expect with a reason, shell.rs's was stale on Windows and is deleted), 24 Windows-only hits (borrow_as_ptr, map_unwrap_or, unnested_or_patterns), and 6 Linux-only hits, found with `scripts/precheck linux` in the container (now passing; one update test #[expect]s verbose_file_reads because it reads through a handle opened before the swap). -D warnings stays in the Makefile. make check and make precheck (8/8) pass. Nothing left unwired.
- 2026-10-18T11:19:50.001Z: Landed on release/0.40.0
