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
---

## Description
ADR 0008 §0 and §7: inventory what a subagent run gets today (its lease is scoped narrower than the home thread's) and bring runs and jobs to the clone model: the home thread's lease unless the home agent narrows it for the task, the home agent's model and workspace, a start from the brief (not the home thread), and search_thread, list_chapters and the thread readers to reach back. A capability change: security.md's five items, tests allowed and refused, residual risk stated.
