---
id: task-106
title: "Audit: lint policy and a workspace [lints] table"
column: review
position: 4
priority: high
tags:
  - audit
  - lints
parentId: epic-12
createdAt: "2026-10-17T19:28:04.724Z"
contract:
  status: delivered
  deliverables:
    - type: docs
      path: docs/audits/0.40/lints.md
      description: Lint policy audit report with a proposed [lints] table
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/lints.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/lints.md && grep -q '^## Leave alone' docs/audits/0.40/lints.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/lints.md
      - grep -q '\[workspace.lints' docs/audits/0.40/lints.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.rs:[0-9]+' docs/audits/0.40/lints.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/lints.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/lints.md; propose the [lints] table in the report, do not add it to Cargo.toml"
    - "Report has exactly these sections: '## Findings' (each lint with count and a file:line sample), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - If a cargo run rewrites ui/src/generated/contract.ts, restore it before committing
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
    pickedUpAt: "2026-10-17T19:30:16.112Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T19:33:57.148Z"
    duration: 221
updatedAt: "2026-10-17T19:33:57.148Z"
relatedFiles:
  - docs/audits/0.40/lints.md
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-app/Cargo.toml
  - crates/hotline-cli/Cargo.toml
  - Makefile
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-17T19:29:50.340Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T19:29:57.364Z"
assignee: claude
---

## Description
Audit: lint policy.

- Run clippy with pedantic (and selected nursery/restriction) over the workspace. For each lint with hits, say: worth adopting / not worth it / adopt later, with its count and a sample.
- Propose a workspace [lints] table (rust + clippy) in Cargo.toml that every crate inherits, plus the order to fix toward it in landable slices.
- Review the 8 too_many_arguments allows and other #[allow]s for whether a refactor removes the need.
- Check rustfmt config and edition consistency.
- Report: docs/audits/0.40/lints.md.
