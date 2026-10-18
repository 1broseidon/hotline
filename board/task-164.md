---
id: task-164
title: "P2-D-4 (E): Present-tense wording and what `make check` runs"
column: review
position: 53
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:08.998Z"
relatedFiles:
  - crates/hotline-core/src/lib.rs
  - Makefile
  - docs/design.md
  - docs/development.md
  - AGENTS.md
updatedAt: "2026-10-18T03:02:23.900Z"
contract:
  status: delivered
  validation:
    commands:
      - grep -q HOTLINE_EXTRA_DESKS docs/development.md
      - grep -q HOTLINE_DEV_SIGN_IDENTITY docs/development.md
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:56:13.058Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T03:02:23.900Z"
    duration: 371
dependsOn:
  - task-121
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:24.387Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:56:01.462Z"
assignee: claude
---

## Description
Phase 2 group E (Docs), plan row D-4 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Present-tense wording and what `make check` runs.** Fix `lib.rs:5`, `Makefile:42` and `design.md:663` (history stays labelled as history). Remove the test counts from development.md and list what `make check` really runs there and in `AGENTS.md:55`. Document `HOTLINE_EXTRA_DESKS` and `HOTLINE_DEV_SIGN_IDENTITY`

- Source: dead-code F10, task 14; tests-and-docs D6, D11, tasks 14 and 17 (env part)
- Files: `crates/hotline-core/src/lib.rs:5`, `Makefile:42`, `docs/design.md:663`, `docs/development.md:108-116,499`, `AGENTS.md:55`
- Collides with: 9 (`lib.rs`)
- Size: XS

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T03:02:23.414Z: [claude] Done on dispatch/task-164, commit 5aa6ca0d. lib.rs header now says what the core holds instead of naming phases; Makefile verify comment no longer says "Phase 0 adds the first"; design.md Phases intro is labelled history and in past tense (design.md:137 "until Phase 1 replaces it" was outside the row's three sentences and is left for D-5 or later). development.md: make check list now matches the Makefile (bun run test, speech engine, both Python suites), test counts gone from "Running it" and "The harness" (desk.rs is described without numbers), HOTLINE_EXTRA_DESKS and HOTLINE_DEV_SIGN_IDENTITY documented after make verify. AGENTS.md:55 and development.md's make check one-liner match. make precheck: 7/7 pass. D-5 (same file) can proceed; the live-test switches from D11 (HOTLINE_LIVE_XAI_KEY etc.) are still undocumented, out of this row's scope.
