---
id: epic-5
title: "Performance: a desk and window that stay fast on long tapes"
type: epic
column: todo
position: 33
priority: high
tags:
  - improvement
  - performance
createdAt: "2026-10-12T01:51:39.966Z"
---

## Description
From the 0.33.4 responsiveness audit. The core folds whole tapes on hot paths and serialises commands; the window re-renders on every streaming frame and remounts long conversations. Children, in rough order of payoff:

- Core I/O: fold cache, roster rows from memory, cheap list calls, per-stream write locks
- Core: answer a socket's commands concurrently
- Window: streaming re-renders, progressive render and virtualization, cached room-wide lists
- Startup: open the window before the core finishes settling
