---
id: task-135
title: "P2-26 (C): `driver/acp.rs` tests"
column: todo
position: 24
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:53.841Z"
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - crates/hotline-core/src/driver/acp/tests.rs
  - crates/hotline-core/src/driver/acp/tests/
updatedAt: "2026-10-18T00:14:54.268Z"
contract:
  status: ready
  validation:
    commands:
      - test -f crates/hotline-core/src/driver/acp/tests.rs
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-128
  - task-115
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:54.128Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 26 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`driver/acp.rs` tests** → `acp/tests.rs` plus `acp/tests/*.rs`

- Source: structure F2, task 12
- Files: `driver/acp.rs`
- Collides with: 27
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
