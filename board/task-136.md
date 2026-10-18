---
id: task-136
title: "P2-27 (C): `driver/acp.rs` code"
column: review
position: 25
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.356Z"
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - crates/hotline-core/src/driver/acp/oauth_proxy.rs
  - crates/hotline-core/src/driver/acp/live.rs
  - crates/hotline-core/src/driver/acp/translate.rs
  - crates/hotline-core/src/driver/acp/session.rs
updatedAt: "2026-10-18T04:54:58.827Z"
contract:
  status: delivered
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/driver/acp.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:36:14.227Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T04:54:58.827Z"
    duration: 1125
dependsOn:
  - task-135
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:55.256Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:36:06.317Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 27 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`driver/acp.rs` code** → `oauth_proxy.rs`, `live.rs`, `translate.rs`, `session.rs`

- Source: structure F2, task 13
- Files: `driver/acp.rs`
- Collides with: 3 (`acp/install.rs` is untouched, but land 3 first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
