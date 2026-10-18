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
updatedAt: "2026-10-18T00:15:56.333Z"
assignee: claude
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-17T22:41:27.659Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:15:56.333Z"
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

## Log
- 2026-10-17T22:48:52.273Z: [claude] Lead plan (claude/opus, 2026-10-17). Every child now has relatedFiles, validation (make precheck + make check for Rust; ui-check for window; a grep/line-count check of its own where the outcome can be measured; move-only C children also prove contract.ts byte-identical against their merge-base) and dependsOn as below. ORDER: A security runs as two serial chains because they share security.md and Cargo.lock: 112 (colleague fields, re-words security.md:683) → 113 (the other three rows + :1426 proof) → 115 (ACP zip, new security row), and Cargo.lock 114 → 115 → 116 → 117 → 123 (B dep surface) → 125 (licenses). bun.lock: 118 → 119 → 124. B: 120 → 121. P2-14 (126, the mod.rs rename) waits on 113, 121, 125, so every Rust branch before it has landed and every Rust task after it depends on it: no Rust branch is open beside it. C: 127 → 128 (contract leaf touches callers everywhere) → then fan-out: 129; 130 → 131, 130+113 → 132; 133; 134; 135 (also after 115) → 136; 137 → 141; 138; 139; 140. Window: 122 → P2-33 split into four siblings (142 App hooks, 172 Settings, 173 Transcript, 174 Teammate) → 157 → 156 (also Rust updater, after 126) → 158 → 159 → 168 → 169 → 170 (also after 124) → 171. D Rust: 143, 144, 155 after 126; 145 after 128; 146 after 134 → 147, 152; 153 after 141; 154 (split: 154 shared Client, then new 175 until()) after 132/133/141/147/152/153. The lint chain 148 → 149 → 150 → 151 → 160 goes after all of C and the targeted D fixes, because autofix touches ~700 sites; 167 (lints table) after 160, 156, 175. E: 162, 163 any time; 161 after 138/139; 164 (touches lib.rs) after 121 and 126; 165 after all C and 164; 166 after 165. Added task-176 last: precheck-full, verify, every F gate in make check, security.md cites only live tests. Window splits skipped the plan's "39 after 33" since clippy cannot touch .tsx.
