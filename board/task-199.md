---
id: task-199
title: "After: timings again, and the plan for splitting the core"
column: in-progress
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
updatedAt: "2026-10-19T08:46:47.051Z"
contract:
  status: in_progress
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
    pickedUpAt: "2026-10-19T08:46:47.051Z"
    reworkCount: 0
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

## Log
- 2026-10-19T09:13:10.586Z: [claude] On dispatch/task-199 (3d938ff7). ADR 0007 gains "## After" and "## Next: splitting the core".

After: Before's commands re-run on fd6ddfd1. The core lib unit went from 40.2s to 31.8s (front end 26.6s to 19.7s). Cold builds: 71.1s to 61.8s from nothing, and 41.7s to 33.5s with the dependencies cached. The incremental floor fell by about a third, though Before did not expect it: a core touch builds in 3.2s (was 4.9s) and test-builds in 8.6s (was 12.4s). A contract edit build is unchanged at 5.0s. The store's own tests build in 0.7s and the contract's in 2.2s. Two command changes: the moved files' new paths, and restoring the file before each probe append, because as written the second append redefines timing_probe and does not compile. scripts/timings-summary now counts the five crates as the workspace's own units and prints each unit's start; this is the dev script change the baseline promised, and no crate changed. The section also has the new module map. linked is now a leaf, and room->mcp (room.rs:283, a pure normalize_servers call) is a new one-link cut. It records that no caller moved, with about 870 core references going through the re-exports. On the test-support feature: yes, it is the pattern for shared fixtures, and why.

Next: a proposal only, in this order. (1) Move room, the pure pieces and RoomHandle down; RoomHandle's five core-typed accessors must leave it. (2) The mcp client out, through rmcp's CredentialStore; the server stays. (3) The voice engines out (speech/realtime, through the existing Speech and Realtime traits); the calls stay. (4) remote, last and conditional. The re-exports are renamed per module as each one leaves, not cleaned up as a stage.

make check, make verify and make precheck (8 legs, 131s) pass in the foreground; contract.ts is byte-identical. The one piece of code-comment drift is noted on epic-16.
