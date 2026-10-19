---
id: task-199
title: "After: timings again, and the plan for splitting the core"
column: todo
position: 7
priority: medium
tags:
  - crate-split
  - measure
assignee: claude
relatedFiles:
  - docs/adr
parentId: epic-16
createdAt: "2026-10-19T06:07:01.477Z"
dependsOn:
  - task-195
  - task-197
  - task-198
updatedAt: "2026-10-19T06:07:13.307Z"
---

## Description
Re-run the baseline's measurements with the same commands and add them to the ADR beside the first ones, plainly: what got faster, what didn't. Then write the next stage as a proposal only: the trait seams that would let voice, remote and mcp leave hotline-core (wire already talks to a RoomHandle), what each seam costs, and the order. No code. George decides from it.
