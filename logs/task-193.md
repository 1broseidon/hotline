---
id: task-193
title: "Baseline: build timings and the module map, in an ADR"
priority: high
tags:
  - crate-split
  - measure
assignee: claude
relatedFiles:
  - Cargo.toml
  - docs/adr
parentId: epic-16
createdAt: "2026-10-19T06:06:19.521Z"
updatedAt: "2026-10-19T06:38:42.731Z"
contract:
  status: delivered
  deliverables:
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: Before timings, module map, leaves, cuts, core cycle, target crate graph
    - type: file
      path: scripts/module-map
      description: Re-runnable script printing crate::X references between top-level modules
  validation:
    commands:
      - test -f docs/adr/0007-split-hotline-core.md
      - grep -q '^## Before' docs/adr/0007-split-hotline-core.md
      - grep -q 'cargo build -p hotline-core' docs/adr/0007-split-hotline-core.md
      - test -x scripts/module-map && scripts/module-map | grep -q websearch
      - git diff --quiet release/0.41.0...HEAD -- crates Cargo.toml Cargo.lock ui
      - python3 -m unittest discover -s scripts -p 'test_*.py'
  constraints:
    - No code changes under crates/, Cargo.toml, Cargo.lock or ui/
    - Each timing run twice, the second kept, on this Mac
  metrics:
    readyAt: "2026-10-19T06:11:03.728Z"
    pickedUpAt: "2026-10-19T06:11:28.590Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T06:37:35.003Z"
    duration: 1566
dispatch:
  agent: claude
  model: sonnet
  effort: high
  by: lead
  at: "2026-10-19T06:11:18.158Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T06:11:20.001Z"
    ended: "2026-10-19T06:37:43.755Z"
    outcome: review
completedAt: "2026-10-19T06:38:42.731Z"
---

## Description
Measure before moving anything, on this Mac (Apple silicon), each run twice and the second kept:
- cold: `cargo clean -p hotline-core && cargo build -p hotline-core --timings` (keep the summary, not the HTML), and the same for `cargo test -p hotline-core --no-run`;
- incremental: touch one file in voice/, one in store.rs, one in contract/, and time `cargo build -p hotline-core` and `cargo test -p hotline-core --no-run` after each;
- note how long hotline-core's own compile takes against its dependencies in the timings.
Write docs/adr/0007-split-hotline-core.md (0007 is the next free number on release/0.41.0): the exact commands, the timings under a heading `## Before` (task-199 adds `## After` beside it with the same commands, so write them so they can be re-run verbatim), the module dependency map (crate::X references between top-level modules, brace imports included; script it as an executable `scripts/module-map`, run from the repo root with no arguments, so later stages re-run it), the leaves, the one-link cuts, the strongly connected core, and the target crate graph for this epic's stages. Settle the name of the leaf-utilities crate there (hotline-support unless it reads wrong) and say it in your final note, so the lead can fix task-196's contract if it changes. No code changes: validation refuses any diff under crates/, Cargo.toml, Cargo.lock or ui/.

## Log
- 2026-10-19T06:37:34.745Z: [claude] Done, on branch dispatch/task-193 (1931d0e9): docs/adr/0007-split-hotline-core.md (## Before, module map, cuts, core, target graph), scripts/module-map (executable, no args), scripts/timings-summary (prints the cargo --timings summary; task-199 adds the new crates to its OWN set), scripts/test_module_map.py. NAME: the leaf-utilities crate stays hotline-support, so task-196's contract needs no change. For later tasks: (1) Before, second runs: cold build 41.7s, cold test --no-run 78.8s; touch/edit of voice/dispatcher.rs, store.rs, contract/command.rs is a flat 5.0s build and 12.4-12.6s test --no-run whichever file, so incremental already clears standards.md's 30s trigger; hotline-core's lib is 40.2s of a 71s empty-target build (front end 26.5s) and in the test build the test-profile lib is 78.5s of 78.8s. About 14% of the lines (23.1k of 163k) move in these five stages, so expect small gains; the ADR says so and task-199 checks it. (2) Cuts, one site each: task-196 paths->linked: move valid_desk_id, valid_persona_id, split, is_linked into paths and re-export from linked (support then needs no hotline-contract at all); task-197 store->thread: Link, ThreadState and AgentBinding are plain types with no ts-rs derive and name only contract, so they move down and thread stays; task-198 tools->driver: CapabilityLease/CapabilityEpoch. Moved sets have no other edge into the core, test code included. (3) The epic text differs from the map: paths, log and linked are one cycle (log->paths is 15 sites; log->credentials is no cut), so skills and log_file are leaves only once paths is. Core is 20 modules, 18 after the cuts. (4) standards.md still says 'No new crates' and forbids single-impl traits to break session/driver/voice/mcp cycles: task-196 should rewrite the first; the second stands for task-199's plan. (5) Rerun scripts/module-map after each stage. (6) python3 -m unittest discover hangs here in test_githooks (mise exec -- node never returns, unrelated to this change); the other four test files pass. For the timings a worktree needs .speech-engine/ copied in (gitignored).
- 2026-10-19T06:38:42.325Z: Landed on release/0.41.0
