---
id: task-128
title: "P2-16 (C): `contract` becomes a leaf"
column: review
position: 17
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:50.224Z"
relatedFiles:
  - crates/hotline-core/src/contract.rs
  - crates/hotline-core/src/thread.rs
  - crates/hotline-core/src/remote.rs
  - crates/hotline-core/src/linked.rs
  - crates/hotline-core/src/imagegen.rs
  - crates/hotline-core/src/spending.rs
updatedAt: "2026-10-18T03:01:58.672Z"
contract:
  status: delivered
  validation:
    commands:
      - "! grep -n 'crate::' crates/hotline-core/src/contract.rs | grep -v 'crate::contract'"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:42:31.207Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T03:01:58.672Z"
    duration: 1167
dependsOn:
  - task-127
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:46.273Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:42:19.691Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 16 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`contract` becomes a leaf.** Move the types it borrows into it, with no re-export left behind. `contract.ts` stays byte-identical

- Source: structure F3, F10, task 3
- Files: `contract.rs`, `thread`, `remote`, `linked`, `imagegen`, `spending`, and their callers
- Collides with: 17
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T03:01:58.175Z: [claude] Done on dispatch/task-128 (commit 8e118ba9). Moved into contract.rs, with no re-export at the old path: ThreadKind, ThreadId, End (thread); BudgetLimits, SpendingSettings plus its custom Deserialize, SpendingSummary (spending); ImageSettings (imagegen); LinkedTeammate, WorkingOn, Profile, Capabilities (LinkedTeammate needs those three), LinkedDelivery (linked); DeviceRole (remote). They sit in four new sections just above "The wire". Inherent impls that do work (ThreadId::stream, Display, ThreadKind::name/parse, DeviceRole::seated, BudgetLimits/SpendingSettings::validate, ImageSettings::is_off) stayed in their own modules, because they reach log/WireRole/OFF. Every caller now imports crate::contract::X, including thread/{link,policy}.rs, remote/client.rs and the cli tests (hotline_core::contract::DeviceRole). The three contract tests that read store output (tape, chapters and preview, search hits) moved to the new store/tests.rs, because they reach log/paths/store. contract.rs keeps the pure wire-shape tests. contract.ts is byte-identical. VALIDATION CONFLICT: the grep check still matches one line, contract.rs:2424 "/// (see [`crate::paths::thread_key`])", a rustdoc link on ThreadId and not a use. ts-rs copies that doc into contract.ts, so rewording it breaks byte-identity. I kept the doc unchanged. Rewording ThreadId's doc in both files needs a task that may change contract.ts. Task 17 (split) should know: the moved types are in sections Threads / Spending and images / Linked desks / Paired devices. make check and make precheck (7/7) pass.
