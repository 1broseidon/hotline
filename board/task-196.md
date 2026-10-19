---
id: task-196
title: "hotline-support: the small leaf utilities as one crate"
column: todo
position: 4
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/credentials.rs
  - crates/hotline-core/src/process_windows.rs
  - crates/hotline-core/src/images.rs
  - crates/hotline-core/src/thumbs.rs
  - crates/hotline-core/src/fence.rs
  - crates/hotline-core/src/room_lock.rs
  - crates/hotline-core/src/log_file.rs
  - crates/hotline-core/src/paths.rs
parentId: epic-16
createdAt: "2026-10-19T06:06:59.870Z"
dependsOn:
  - task-194
updatedAt: "2026-10-19T07:50:18.405Z"
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-support/Cargo.toml
      description: The leaf-utilities crate, on hotline-contract at most
  validation:
    commands:
      - test -f crates/hotline-support/src/lib.rs
      - for f in credentials process_windows images thumbs fence room_lock log_file paths; do test ! -e crates/hotline-core/src/$f.rs || { echo "$f.rs still in hotline-core"; exit 1; }; done
      - "! grep -qE '^hotline-(core|store|tools|websearch)' crates/hotline-support/Cargo.toml"
      - grep -A1 '^\[lints\]' crates/hotline-support/Cargo.toml | grep -q 'workspace = true'
      - cargo test -p hotline-support -- --list 2>/dev/null | grep -q room_lock
      - grep -q hotline-support docs/standards.md && grep -q hotline-support AGENTS.md
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Moves only: no behaviour change"
    - One crate, not seven
    - SAFETY comments kept on unsafe code
    - contract.ts byte-identical
  metrics:
    readyAt: "2026-10-19T06:11:04.263Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T07:50:16.813Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T07:50:18.405Z"
---

## Description
One crate for the leaves that hold no Hotline state: credentials (atomic_write and friends, credentials/), process_windows, images, thumbs (reaches images), fence, room_lock, log_file (reaches paths; its harness=false integration test crates/hotline-core/tests/log_file.rs and the [[test]] entry move with it), and paths once its one link to linked (is_linked, a string test) moves into paths itself or into hotline-contract. Use the name ADR 0007 settled (hotline-support unless it says otherwise); keep it one crate, not seven. room_lock is the one-writer-per-data-dir guard (desk.lock): its tests move with it and still prove a second desk is refused. Unsafe code keeps its SAFETY comments.

Lead's note: this runs before task-197 and task-198 (both need it) and alone, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md).

From task-194: hotline-core keeps `pub use hotline_contract as contract`, so callers still say crate::contract; hotline-contract depends on no hotline crate. Its first landing hit the 15-minute `make check` limit, most likely a cold target/ in the landing checkout rather than a hang (make check ran in 4m11s warm). Don't add to the cold-build cost without cause: no new dev-dependencies or features beyond what the moved code already used.
