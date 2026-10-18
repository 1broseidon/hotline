---
id: epic-13
title: "0.40 quality Phase 2: security, dead code, restructure, fixes, docs and gates (approved A–F)"
type: epic
priority: high
tags:
  - quality
  - "0.40"
  - phase2
createdAt: "2026-10-17T22:40:41.476Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T11:53:29.644Z"
assignee: claude
completedAt: "2026-10-18T11:53:29.644Z"
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

## Child Tasks
Summary: 65/65 children completed.
- task-112: Colleagues see each other's goal, agent, model and capabilities (computer, web search, images, background work, MCP server names) (completed)
- task-113: P2-1 (A): security.md proves what it cites (completed)
- task-114: P2-2 (A): Patch rustls (completed)
- task-115: P2-3 (A): Patch ACP ZIP extraction (F1) (completed)
- task-116: P2-4 (A): Tauri updater ZIP path (completed)
- task-117: P2-5 (A): sherpa build ZIP path (completed)
- task-118: P2-6 (A): Patch source-map-js (completed)
- task-119: P2-7 (A): Ship a Mermaid bundle with KaTeX fixed (completed)
- task-120: P2-8 (B): Rust dead code (completed)
- task-121: P2-9 (B): Narrow `lib.rs` (completed)
- task-122: P2-10 (B): Window dead code (completed)
- task-123: P2-11 (B): Dependency surface (completed)
- task-124: P2-12 (B): Deduplicate `@tauri-apps/api` (completed)
- task-125: P2-13 (B): Declare crate licenses (completed)
- task-126: P2-14 (C): `foo.rs` + `foo/` (completed)
- task-127: P2-15 (C): One test-file naming style (completed)
- task-128: P2-16 (C): `contract` becomes a leaf (completed)
- task-129: P2-17 (C): Split `contract.rs` (completed)
- task-130: P2-18–22 (C): `session/mod.rs` in five parts: (completed)
- task-131: P2-22a (C): `Room` constructors: (completed)
- task-132: P2-23 (C): `session/tests.rs` (completed)
- task-133: P2-24 (C): `wire/tests.rs` (completed)
- task-134: P2-25 (C): `remote/tests.rs` (completed)
- task-135: P2-26 (C): `driver/acp.rs` tests (completed)
- task-136: P2-27 (C): `driver/acp.rs` code (completed)
- task-137: P2-28 (C): `voice/mod.rs` (completed)
- task-138: P2-29 (C): `wire/mod.rs` (completed)
- task-139: P2-30 (C): `wire/commands.rs`: (completed)
- task-140: P2-31 (C): The six other over-budget files (completed)
- task-141: P2-32 (C): The remaining big test files: (completed)
- task-142: P2-33a (C): Window split, move-only: App() → useNavigation, useSeen, useWindowHotkeys (completed)
- task-143: P2-34 (D): One boot that returns errors (completed)
- task-144: P2-35 (D): Discovery client is fallible (completed)
- task-145: P2-36 (D): Lock recovery (completed)
- task-146: P2-37 (D): One `ANSWER` for the remote tests, (completed)
- task-147: P2-38 (D): The remote harness connects over loopback (completed)
- task-148: P2-39 (D): Lint autofix slice (completed)
- task-149: P2-40 (D): By-hand lints and reasoned invariants (completed)
- task-150: P2-41 (D): Argument groups (completed)
- task-151: P2-42 (D): `result_large_err`: all 13 become `#[expect(…, reason)]` (completed)
- task-152: P2-43 (D): Paused clock (completed)
- task-153: P2-44 (D): Honest tests (completed)
- task-154: P2-45a (D): Shared test plumbing: one bounded test Client in tests/common (completed)
- task-155: P2-46 (D): Search failures get a kind (completed)
- task-156: P2-47 (D): Updater and settings errors use Refusal details (completed)
- task-157: P2-48 (D): Window fixes that Biome needs first (completed)
- task-158: P2-49 (D): Window plumbing (completed)
- task-159: P2-50 (D): a11y pass: (completed)
- task-160: P2-51 (D): Audit lenses, with no table entry (completed)
- task-161: P2-D-1 (E): wire.md matches the code (completed)
- task-162: P2-D-2 (E): Server docs (completed)
- task-163: P2-D-3 (E): threads.md and sessions.md name live code (completed)
- task-164: P2-D-4 (E): Present-tense wording and what `make check` runs (completed)
- task-165: P2-D-5 (E): One module map, (completed)
- task-166: P2-D-6 (E): `docs/standards.md` (completed)
- task-167: P2-52 (F): The `[workspace.lints]` table in one change (completed)
- task-168: P2-53 (F): Biome for lint only (completed)
- task-169: P2-54 (F): The window's gates (completed)
- task-170: P2-55 (F): The dependency gates (completed)
- task-171: P2-56 (F): The license gate (completed)
- task-172: P2-33b (C): Window split, move-only: Settings.tsx → components/settings/ (completed)
- task-173: P2-33c (C): Window split, move-only: Transcript.tsx (folds to transcriptFolds.ts first) (completed)
- task-174: P2-33d (C): Window split, move-only: Teammate.tsx (completed)
- task-175: P2-45b (D): One until(what, cond) with one deadline, replacing the T8 sleep-then-assert sites (completed)
- task-176: Phase 2 holds together: precheck-full, verify, and every gate in make check (completed)
