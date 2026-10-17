---
id: task-148
title: "P2-39 (D): Lint autofix slice"
column: todo
position: 37
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:00.635Z"
relatedFiles:
  - crates/hotline-core/src/
  - crates/hotline-core/tests/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/
  - crates/hotline-cli/tests/
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-129
  - task-131
  - task-132
  - task-133
  - task-136
  - task-138
  - task-139
  - task-140
  - task-141
  - task-143
  - task-144
  - task-145
  - task-147
  - task-152
  - task-153
  - task-155
  - task-175
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 39 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Lint autofix slice.** The fifteen "[1]" lints from lints.md, one commit per lint, followed by an unfiltered `cargo test -p hotline-core`. Prove each slice clean with a one-off `cargo clippy -- -W clippy::<lint>`; the table comes later (C4)

- Source: lints, task 2
- Files: about 700 sites workspace-wide
- Collides with: every split in C: land after 33
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
