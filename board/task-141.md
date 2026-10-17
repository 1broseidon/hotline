---
id: task-141
title: "P2-32 (C): The remaining big test files:"
column: todo
position: 30
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:56.966Z"
relatedFiles:
  - crates/hotline-core/src/voice/tests.rs
  - crates/hotline-core/tests/voice.rs
  - crates/hotline-core/tests/desk.rs
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - for f in crates/hotline-core/src/voice/tests.rs crates/hotline-core/tests/voice.rs crates/hotline-core/tests/desk.rs; do test ! -f $f || test $(wc -l < $f) -le 2000 || exit 1; done
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-137
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 32 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The remaining big test files:** `voice/tests.rs`, `tests/voice.rs`, `tests/desk.rs`

- Source: structure F6, task 18
- Files: those files
- Collides with: 43, 44, 45
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
