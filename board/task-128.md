---
id: task-128
title: "P2-16 (C): `contract` becomes a leaf"
column: todo
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
updatedAt: "2026-10-18T00:14:46.408Z"
contract:
  status: ready
  validation:
    commands:
      - "! grep -n 'crate::' crates/hotline-core/src/contract.rs | grep -v 'crate::contract'"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-127
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:46.273Z"
  until: done
  tries: 0
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
