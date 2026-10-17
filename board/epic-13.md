---
id: epic-13
title: "0.40 quality Phase 2: security, dead code, restructure, fixes, docs and gates (approved A–F)"
type: epic
column: todo
position: 1
priority: high
tags:
  - quality
  - "0.40"
  - phase2
createdAt: "2026-10-17T22:40:41.476Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T22:41:27.788Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-17T22:41:27.659Z"
  until: done
  role: lead
assignee: claude
---

## Description
Phase 2 of the 0.40 quality checkpoint. George approved all of groups A–F on 2026-10-17. The authoritative plan is docs/audits/0.40/README.md on release/0.40.0 ("Phase 2 order"): its six reports, the Contradictions with their resolutions, and the Standards. One child per plan row; titles keep the plan's numbers.

**Decisions (George, 2026-10-17):**
- Standards approved as written in README § Standards (D-6 goes ahead; follow them in every task).
- security.md:683: a colleague's goal stays visible. Document it, and widen what colleagues see per task-112 (agent, model, capabilities). Task-112 is part of this epic and pairs with P2-1.
- D's error-behaviour fixes are approved.
- `Failure` stays unboxed: all 13 result_large_err become expects (C6).

**Order (from the plan):** A security → B dead code → C move-only (renames first, then the hot spots) → D fixes in the moved files → F gates last. E docs runs alongside C/D unless a collision is named; D-5 and D-6 come after C.
- Within a group, run tasks in parallel only where "Collides with" names no open task.
- **P2-14 (foo.rs + foo/) lands with no other Rust branch open.**
- Every task is one landable commit, or the short series it names. Move-only commits are pure moves (`git diff -M` near 100%) so blame survives.
- Each task ends with `make precheck`; `make precheck-full` before the epic is called done.
- The git hooks are on (core.hooksPath .githooks). Never `--no-verify`.
- No behaviour change outside D and A, no wire/contract change (contract.ts byte-identical through C), and Windows/Linux stay green.

Done when every child has landed on release/0.40.0, `make precheck-full` passes, and the gates in F are in make check.
