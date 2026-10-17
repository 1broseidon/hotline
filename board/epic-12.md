---
id: epic-12
title: "0.40 quality checkpoint: audit maintainability, structure, dead code, errors, lints, window, tests and docs — then fix what George approves"
type: epic
column: todo
position: 0
priority: high
tags:
  - quality
  - audit
  - "0.40"
createdAt: "2026-10-17T19:28:02.163Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T19:28:02.674Z"
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
