---
id: task-106
title: "Audit: lint policy and a workspace [lints] table"
column: todo
position: 4
priority: high
tags:
  - audit
  - lints
parentId: epic-12
createdAt: "2026-10-17T19:28:04.724Z"
---

## Description
Audit: lint policy.

- Run clippy with pedantic (and selected nursery/restriction) over the workspace. For each lint with hits, say: worth adopting / not worth it / adopt later, with its count and a sample.
- Propose a workspace [lints] table (rust + clippy) in Cargo.toml that every crate inherits, plus the order to fix toward it in landable slices.
- Review the 8 too_many_arguments allows and other #[allow]s for whether a refactor removes the need.
- Check rustfmt config and edition consistency.
- Report: docs/audits/0.40/lints.md.
