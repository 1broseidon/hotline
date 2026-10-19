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
updatedAt: "2026-10-19T06:10:21.788Z"
---

## Description
Re-run the baseline's measurements with the same commands and add them to docs/adr/0007-split-hotline-core.md beside the first ones under a heading `## After`, plainly: what got faster, what didn't. Re-run scripts/module-map and put the new map beside the old. Then write the next stage under a heading `## Next: splitting the core`, as a proposal only: the trait seams that would let voice, remote and mcp leave hotline-core (wire already talks to a RoomHandle), what each seam costs, and the order. No code. George decides from it.

This is also the epic's integration check: with every crate landed, prove the pieces work together against epic-16's description of done. Every new crate (hotline-contract, hotline-support or the name the ADR chose, hotline-websearch, hotline-store, hotline-tools) has `[lints] workspace = true`, a license, and a crate-level doc saying what it owns and must never depend on; docs/standards.md and AGENTS.md name each; hotline-core has no Tauri in its tree; make check, make verify (the headless harnesses over the wire) and make precheck pass on the integrated branch, and contract.ts is byte-identical. Fix doc drift you find (docs only); any code gap goes in a note on epic-16 for the lead, not into this task.
