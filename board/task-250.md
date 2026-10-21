---
id: task-250
title: "The fast gate: a required Linux check under 10 min; the slower OS checks where George decides"
column: backlog
position: 50
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:28.635Z"
---

## Description
Starts after the four changes above land, with their numbers. George decides where the Windows and macOS checks run (every dispatch, main only, nightly, or before a cut).
Done when:
- A Linux job (fmt, clippy, nextest, the dependency gates, the window checks) finishes in under 10 min and is the check every dispatch waits on.
- The Windows and macOS checks run where George decided, and still run before every release cut.
- The PR records the gate's time over five runs.
