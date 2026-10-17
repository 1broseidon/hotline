---
id: epic-12
title: "0.40 quality checkpoint: audit maintainability, structure, dead code, errors, lints, window, tests and docs — then fix what George approves"
type: epic
priority: high
tags:
  - quality
  - audit
  - "0.40"
createdAt: "2026-10-17T19:28:02.163Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T20:01:55.225Z"
assignee: claude
completedAt: "2026-10-17T20:01:55.225Z"
---

## Description
0.40 is a quality checkpoint (George, 2026-10-17): a complete audit of the desk repo for maintainability, best practices, code quality, dead code, and Rust file and folder structure. Then the fixes George approves. No behaviour changes.

Baseline (release/0.40.0, 2026-10-17):
- ~166k lines of Rust: hotline-core 159k, app 4k, cli 3k.
- Largest files: driver/acp.rs 5.8k, session/mod.rs 5.6k, contract.rs 4.3k, voice/mod.rs 3k, session/sides.rs 2.7k, driver/rig.rs 2.7k, wire/mod.rs 2.5k. Test files to 7k (session/tests.rs, wire/tests.rs). UI: Settings.tsx, Transcript.tsx and Teammate.tsx each ~2k.
- Module layout is mixed (foo.rs + foo/ vs foo/mod.rs), and tests come in three patterns.
- clippy is clean at default; ~1,000 pedantic warnings (missing # Errors docs, must_use, match style, needless pass-by-value).
- 26 #[allow]s, 5 of them dead_code. ~2,800 unwrap/expect (inflated by in-file test modules).
- No unused-dependency check, no cargo deny/audit, no TS dead-code tool. Two flaky tests found 2026-10-17 (fixed).

**Phase 1 (this lead): six read-only audits.** Each writes one report at docs/audits/0.40/<area>.md, the only file it changes. A report holds:
- findings ranked by severity × effort, each with file:line evidence;
- what to leave alone and why;
- a proposed list of Phase 2 tasks, each small, mechanical where possible, and landable alone behind make check.

Tools may be run ad hoc (cargo machete, cargo +nightly udeps, cargo deny, cargo tree -d, npx knip, clippy pedantic) but are not added to the repo in Phase 1.

**Phase 2** is filed from the reports after George reviews them. It covers:
- docs/standards.md;
- move-only commits for file splits and layout, so blame survives;
- dead code removal;
- workspace [lints] plus fixes;
- the tools added to make check.

Rules: no behaviour change, no public wire or contract change, Windows and Linux CI stay green.

Out of scope: hotline-mobile (a later pass); epic-9 (moved off 0.40).

## Log
- 2026-10-17T19:29:43.627Z: [claude] Plan (lead, 2026-10-17): Phase 1 is six independent read-only audits (task-103..108). Each commits only docs/audits/0.40/<area>.md, so none collide and none wait on another. Each contract checks four things: the report exists with ≥60 lines; it has sections '## Findings', '## Leave alone' and '## Phase 2 tasks'; it cites file:line evidence; and the diff against release/0.40.0 touches no other file (this also keeps a cargo-rewritten generated/contract.ts out). task-109 runs last, after all six. It merges their Phase 2 lists into one ordered, deduped plan with contradictions and standards notes, in docs/audits/0.40/README.md, and George approves from that. Assignments, 3 at a time: first the compile-heavy Rust audits, 103 structure (claude opus), 105 errors (codex gpt-6.1-sol) and 106 lints (claude sonnet); then 104 dead code (codex gpt-6.1-sol), 107 window (claude sonnet) and 108 tests and docs (claude opus); then 109 (claude opus). Phase 2 is not filed until George reviews the README.

## Child Tasks
Summary: 7/7 children completed.
- task-103: Audit: Rust structure and module layout, with a target tree and file splits (completed)
- task-104: Audit: dead code and dependencies (Rust and window) (completed)
- task-105: Audit: error handling and reachable panics (completed)
- task-106: Audit: lint policy and a workspace [lints] table (completed)
- task-107: Audit: the window — dead exports, big components, lint setup (completed)
- task-108: Audit: tests (slow, flaky, duplicated) and docs drift (completed)
- task-109: Audit synthesis: one ordered Phase 2 plan from the six 0.40 reports (completed)
