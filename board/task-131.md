---
id: task-131
title: "P2-22a (C): `Room` constructors:"
column: todo
position: 20
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:51.759Z"
relatedFiles:
  - crates/hotline-core/src/session.rs
  - crates/hotline-core/src/session/start.rs
updatedAt: "2026-10-18T00:14:49.793Z"
contract:
  status: ready
  validation:
    commands:
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-130
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:49.657Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 22a in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`Room` constructors:** one `Room::new` with options, or leave them. Its own commit after 22, not inside a move (C16)

- Source: structure F1
- Files: `session.rs`, `session/start.rs`
- Collides with: 22
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
