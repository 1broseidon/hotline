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
updatedAt: "2026-10-19T08:46:38.918Z"
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: After timings beside Before, the new module map, and the proposal for the core's seams
  validation:
    commands:
      - grep -q '^## Before' docs/adr/0007-split-hotline-core.md && grep -q '^## After' docs/adr/0007-split-hotline-core.md && grep -q '^## Next' docs/adr/0007-split-hotline-core.md
      - for c in hotline-contract hotline-support hotline-websearch hotline-store hotline-tools; do test -f crates/$c/src/lib.rs && grep -A1 '^\[lints\]' crates/$c/Cargo.toml | grep -q 'workspace = true' && grep -q license crates/$c/Cargo.toml && grep -q $c docs/standards.md && grep -q $c AGENTS.md || { echo "$c incomplete"; exit 1; }; done
      - "! cargo tree -p hotline-core -e normal | grep -qi tauri"
      - git diff --quiet release/0.41.0...HEAD -- crates Cargo.toml Cargo.lock ui
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - make verify
      - make precheck
  constraints:
    - "No code: docs only"
    - Same measurement commands as the baseline
    - The core-split stage is a proposal for George, not started
  metrics:
    readyAt: "2026-10-19T06:11:05.319Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T08:46:37.115Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T08:46:38.918Z"
---

## Description
Re-run the baseline's measurements with the same commands and add them to docs/adr/0007-split-hotline-core.md beside the first ones under a heading `## After`, plainly: what got faster, what didn't. Re-run scripts/module-map and put the new map beside the old. Then write the next stage under a heading `## Next: splitting the core`, as a proposal only: the trait seams that would let voice, remote and mcp leave hotline-core (wire already talks to a RoomHandle), what each seam costs, and the order. No code. George decides from it.

This is also the epic's integration check: with every crate landed, prove the pieces work together against epic-16's description of done. Every new crate (hotline-contract, hotline-support or the name the ADR chose, hotline-websearch, hotline-store, hotline-tools) has `[lints] workspace = true`, a license, and a crate-level doc saying what it owns and must never depend on; docs/standards.md and AGENTS.md name each; hotline-core has no Tauri in its tree; make check, make verify (the headless harnesses over the wire) and make precheck pass on the integrated branch, and contract.ts is byte-identical. Fix doc drift you find (docs only); any code gap goes in a note on epic-16 for the lead, not into this task.

From the moves (task-194 through task-198, all landed on release/0.41.0): hotline-core re-exports each moved crate under its old module name (crate::contract, crate::paths, crate::store, crate::tools, crate::websearch), so no caller moved; the ADR's After section should say so, and the Next section can count those re-exports as a cleanup cost. hotline-websearch exposes its mock search servers only behind a `test-support` feature that hotline-core turns on for its tests; say in the ADR whether that pattern is how later crates should share test fixtures. Run `make check`, `make verify` and `make precheck` in the foreground (one call each, long timeout) and end your turn only once all pass and the task is in Review; ending while they run in the background loses the hand-off.
