---
id: task-221
title: "Subagents are clones of the home agent: same permission set, a focused start, reaching back with the same tools"
column: todo
position: 17
priority: high
tags:
  - threads
  - security
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-core/src/driver
  - docs/security.md
parentId: epic-20
createdAt: "2026-10-19T19:59:33.081Z"
dependsOn:
  - task-220
updatedAt: "2026-10-19T20:03:14.742Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: a subagent and a job run under the home thread's lease (and a narrowed one when the home agent narrows it), allowed and refused, and reach back with search_thread and list_chapters
    - type: docs
      path: docs/security.md
      description: the clone model's default, grant source, enforcement points, tests and residual risk
  validation:
    commands:
      - make check
  constraints:
    - "A capability change: follow docs/security.md"
    - A clone's lease is never wider than the home thread's
  metrics:
    readyAt: "2026-10-19T20:03:14.742Z"
---

## Description
ADR 0008 §0 and §7: inventory what a subagent run gets today (its lease is scoped narrower than the home thread's) and bring runs and jobs to the clone model: the home thread's lease unless the home agent narrows it for the task, the home agent's model and workspace, a start from the brief (not the home thread), and search_thread, list_chapters and the thread readers to reach back. A capability change: security.md's five items, tests allowed and refused, residual risk stated.
