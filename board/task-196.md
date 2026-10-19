---
id: task-196
title: "hotline-support: the small leaf utilities as one crate"
column: review
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
updatedAt: "2026-10-19T08:02:16.093Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T07:56:09.596Z"
    reworkCount: 1
    deliveredAt: "2026-10-19T08:02:10.270Z"
    duration: 361
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T07:55:57.760Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T07:56:00.115Z"
    ended: "2026-10-19T08:02:16.093Z"
    outcome: review
---

## Description
One crate for the leaves that hold no Hotline state: credentials (atomic_write and friends, credentials/), process_windows, images, thumbs (reaches images), fence, room_lock, log_file (reaches paths; its harness=false integration test crates/hotline-core/tests/log_file.rs and the [[test]] entry move with it), and paths once its one link to linked (is_linked, a string test) moves into paths itself or into hotline-contract. Use the name ADR 0007 settled (hotline-support unless it says otherwise); keep it one crate, not seven. room_lock is the one-writer-per-data-dir guard (desk.lock): its tests move with it and still prove a second desk is refused. Unsafe code keeps its SAFETY comments.

Lead's note: this runs before task-197 and task-198 (both need it) and alone, because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md).

From task-194: hotline-core keeps `pub use hotline_contract as contract`, so callers still say crate::contract; hotline-contract depends on no hotline crate. Its first landing hit the 15-minute `make check` limit, most likely a cold target/ in the landing checkout rather than a hang (make check ran in 4m11s warm). Don't add to the cold-build cost without cause: no new dev-dependencies or features beyond what the moved code already used.

## Log
- 2026-10-19T07:55:56.948Z: [claude] [lead] The last run ended its turn while `make check` was still running in the background, so nothing reached Review. The move is committed on dispatch/task-196; don't redo it. Run `make check` and then `make precheck` in the foreground (one call each, with a long timeout), fix anything they find, check the contract's validation commands, and end your turn only once they have passed and the task is in Review.
- 2026-10-19T08:02:13.029Z: [claude] On dispatch/task-196 (54e9f6b7 the move, ebe6d1cf the Cargo.lock that drops keyring from hotline-core, since only hotline-support uses it now). In the foreground here: make check passes in 4m02s and make precheck in 99s (all 8 legs). contract.ts is byte-identical to release/0.41.0, every one of the contract's validation commands passes, the 22 unsafe/SAFETY lines are all still there, and the room_lock tests (a second writer is refused) run under hotline-support. For task-197/198: hotline-support depends on no Hotline crate; hotline-core re-exports each module under its old name (crate::paths, crate::credentials, …), so no caller moved; is_linked's string grammar now lives in paths. Nothing is left unwired.
