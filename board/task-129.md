---
id: task-129
title: "P2-17 (C): Split `contract.rs`"
column: todo
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
updatedAt: "2026-10-18T03:50:37.630Z"
contract:
  status: ready
  validation:
    commands:
      - test -d crates/hotline-core/src/contract
      - "! grep -rn 'crate::' crates/hotline-core/src/contract.rs crates/hotline-core/src/contract/ | grep -v 'crate::contract'"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:47.398Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:50:37.630Z"
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
