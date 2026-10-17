---
id: task-165
title: "P2-D-5 (E): One module map,"
column: todo
position: 54
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:09.515Z"
relatedFiles:
  - docs/development.md
  - docs/design.md
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - "! grep -n 'defined once in contract.rs' docs/development.md docs/design.md"
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-164
  - task-129
  - task-131
  - task-132
  - task-133
  - task-136
  - task-138
  - task-139
  - task-140
  - task-141
  - task-142
  - task-172
  - task-173
  - task-174
---

## Description
Phase 2 group E (Docs), plan row D-5 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One module map,** in `docs/development.md`, matching the tree after C, with `design.md` linking to it. Correct the "defined once in contract.rs" line

- Source: structure F11, task 21
- Files: `docs/development.md:45-72,396`, `docs/design.md:634-660`
- Collides with: all of C (land last); D-4 (same file)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
