---
id: task-130
title: "P2-18–22 (C): `session/mod.rs` in five parts:"
column: in-progress
position: 19
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:51.248Z"
relatedFiles:
  - crates/hotline-core/src/session.rs
  - crates/hotline-core/src/session/preamble.rs
  - crates/hotline-core/src/session/asking.rs
  - crates/hotline-core/src/session/secrets.rs
  - crates/hotline-core/src/session/computer.rs
  - crates/hotline-core/src/session/prompt.rs
  - crates/hotline-core/src/session/driver_info.rs
  - crates/hotline-core/src/session/start.rs
  - crates/hotline-core/src/session/chapters.rs
updatedAt: "2026-10-18T03:50:53.522Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/session.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:50:53.522Z"
    reworkCount: 0
dependsOn:
  - task-128
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:48.514Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:50:39.278Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 18–22 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`session/mod.rs` in five parts:** preamble and asking; secrets; computer; prompt and driver_info; start and chapters

- Source: structure F1, tasks 5–9
- Files: `session.rs`, `session/{preamble,asking,secrets,computer,prompt,driver_info,start,chapters}.rs`
- Collides with: 23; 40 (expects in `session/mod.rs:813,2630`)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
