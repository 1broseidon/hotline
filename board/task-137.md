---
id: task-137
title: "P2-28 (C): `voice/mod.rs`"
column: review
position: 26
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.875Z"
relatedFiles:
  - crates/hotline-core/src/voice.rs
  - crates/hotline-core/src/voice/call.rs
  - crates/hotline-core/src/voice/input.rs
  - crates/hotline-core/src/voice/turn.rs
  - crates/hotline-core/src/voice/synthesis.rs
updatedAt: "2026-10-18T04:35:56.010Z"
contract:
  status: delivered
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/voice.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:14:43.476Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T04:35:56.010Z"
    duration: 1273
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:56.374Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:14:28.326Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 28 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`voice/mod.rs`** → `call.rs`, `input.rs`, `turn.rs`, `synthesis.rs`

- Source: structure F4, task 14
- Files: `voice.rs`, `voice/*`
- Collides with: 41 (`start_call`, `speak_sentence`), 40
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
