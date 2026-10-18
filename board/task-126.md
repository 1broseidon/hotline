---
id: task-126
title: "P2-14 (C): `foo.rs` + `foo/`"
column: in-progress
position: 15
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:49.183Z"
relatedFiles:
  - crates/hotline-core/src/computer/mod.rs
  - crates/hotline-core/src/driver/mod.rs
  - crates/hotline-core/src/imagegen/mod.rs
  - crates/hotline-core/src/log/mod.rs
  - crates/hotline-core/src/mcp/mod.rs
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/session/mod.rs
  - crates/hotline-core/src/skills/mod.rs
  - crates/hotline-core/src/store/mod.rs
  - crates/hotline-core/src/thread/mod.rs
  - crates/hotline-core/src/tools/mod.rs
  - crates/hotline-core/src/voice/mod.rs
  - crates/hotline-core/src/voice/realtime/mod.rs
  - crates/hotline-core/src/voice/speech/mod.rs
  - crates/hotline-core/src/websearch/mod.rs
  - crates/hotline-core/src/wire/mod.rs
updatedAt: "2026-10-18T02:08:08.383Z"
contract:
  status: in_progress
  validation:
    commands:
      - "! git ls-files crates | grep -E '/src/(.*/)?mod\\.rs$'"
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:08:08.383Z"
    reworkCount: 0
dependsOn:
  - task-113
  - task-121
  - task-125
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:44.049Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:06:57.568Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 14 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`foo.rs` + `foo/`.** `git mv` all 16 `mod.rs` files; `git diff -M` shows 100% similarity

- Source: structure F9, task 1
- Files: the 16 `mod.rs`
- Collides with: everything under `src/`: land with no other Rust branch open
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
