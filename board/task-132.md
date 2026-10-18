---
id: task-132
title: "P2-23 (C): `session/tests.rs`"
column: todo
position: 21
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:52.292Z"
relatedFiles:
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/session/tests/
updatedAt: "2026-10-18T04:51:05.228Z"
contract:
  status: ready
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/session/tests.rs) -le 2000
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-130
  - task-113
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:50.788Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:51:05.228Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 23 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`session/tests.rs`** → fixtures plus `session/tests/*.rs`, 131 tests before and after

- Source: structure F6, task 10
- Files: `session/tests.rs`
- Collides with: 1 (new test: land 1 first), 45
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
