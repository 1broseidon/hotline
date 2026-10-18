---
id: task-133
title: "P2-24 (C): `wire/tests.rs`"
column: in-progress
position: 22
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:52.807Z"
relatedFiles:
  - crates/hotline-core/src/wire/tests.rs
  - crates/hotline-core/src/wire/tests/
updatedAt: "2026-10-18T03:50:52.207Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/wire/tests.rs) -le 2000
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:50:52.207Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:51.905Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:50:41.767Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 24 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`wire/tests.rs`** → fixtures plus `wire/tests/*.rs`, 126 tests

- Source: structure F6, task 11
- Files: `wire/tests.rs`
- Collides with: 37, 45
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
