---
id: task-129
title: "P2-17 (C): Split `contract.rs`"
column: review
position: 18
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:50.735Z"
relatedFiles:
  - crates/hotline-core/src/contract.rs
  - crates/hotline-core/src/contract/
updatedAt: "2026-10-18T04:21:36.800Z"
contract:
  status: delivered
  validation:
    commands:
      - test -d crates/hotline-core/src/contract
      - "! grep -rn 'crate::' crates/hotline-core/src/contract.rs crates/hotline-core/src/contract/ | grep -v 'crate::contract'"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:50:54.712Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T04:17:06.270Z"
    duration: 1572
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:47.398Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:21:36.800Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 17 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Split `contract.rs`** into the nine submodules, re-exported. `contract.ts` stays byte-identical

- Source: structure F3, task 4
- Files: `contract.rs` → `contract/*.rs`
- Collides with: 16
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:17:05.908Z: [claude] Done on dispatch/task-129 (27d4fbb9, bc5b2562): contract.rs is now the module root with nine submodules re-exported by `pub use` (command, computer, credentials, payloads, persona, replies, session, transcript, views), plus contract/tests.rs. Every crate::contract path still works and contract.ts is byte-identical. make check and make precheck (7/7) pass. One validation check still fails, the same one task-128 hit: the grep matches views.rs:273, the ThreadId rustdoc link "[`crate::paths::thread_key`]". That is a doc link, not an import. ts-rs copies it into contract.ts, so rewording it would break byte-identity, and George reverted that reword when landing 128. I kept it as it is. If the line should change, that needs a task that is allowed to change contract.ts, or a grep that skips `///` lines. Nothing is left unwired.
- 2026-10-18T04:21:36.105Z: Landing sent back (1 of 1): with release/0.40.0 brought in, `! grep -rn 'crate::' crates/hotline-core/src/contract.rs crates/hotline-core/src/contract/ | grep -v 'crate::contract'` failed: crates/hotline-core/src/contract/views.rs:273:/// (see [`crate::paths::thread_key`]) for a pair.. Fix it on this branch, commit, and run the checks again.
