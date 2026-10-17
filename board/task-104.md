---
id: task-104
title: "Audit: dead code and dependencies (Rust and window)"
column: review
position: 2
priority: high
tags:
  - audit
  - dead-code
parentId: epic-12
createdAt: "2026-10-17T19:28:03.692Z"
contract:
  status: done
  deliverables:
    - type: docs
      path: docs/audits/0.40/dead-code-and-deps.md
      description: Dead code and dependencies audit report
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/dead-code-and-deps.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/dead-code-and-deps.md && grep -q '^## Leave alone' docs/audits/0.40/dead-code-and-deps.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/dead-code-and-deps.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.(rs|ts|tsx|toml|json):[0-9]+' docs/audits/0.40/dead-code-and-deps.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/dead-code-and-deps.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/dead-code-and-deps.md; run cargo machete/udeps/deny/tree -d and npx knip ad hoc, add none to the repo"
    - "Report has exactly these sections: '## Findings' (ranked severity x effort, each with file:line evidence), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - If a cargo run rewrites ui/src/generated/contract.ts, restore it before committing
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
    pickedUpAt: "2026-10-17T19:35:39.904Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T19:43:13.785Z"
    duration: 454
updatedAt: "2026-10-17T19:43:38.450Z"
relatedFiles:
  - docs/audits/0.40/dead-code-and-deps.md
  - Cargo.toml
  - crates/hotline-core/Cargo.toml
  - crates/hotline-app/Cargo.toml
  - crates/hotline-cli/Cargo.toml
  - ui/package.json
  - scripts
  - packaging
dispatch:
  agent: codex
  model: gpt-6.1-sol
  effort: high
  by: lead
  at: "2026-10-17T19:35:04.741Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T19:35:07.229Z"
    ended: "2026-10-17T19:43:27.714Z"
    outcome: review
assignee: codex
---

## Description
Audit: dead code and dependencies.

- Unused pub items and modules, all 5 #[allow(dead_code)] (justify each or remove), cfg-gated code that never builds, unused features.
- Stale scripts/, packaging/, docs and fixtures.
- Unused dependencies (cargo machete; cargo +nightly udeps if available), duplicate crate versions (cargo tree -d), heavy deps with a lighter alternative, cargo deny advisories/licenses/bans.
- The same for ui/package.json.
- Report: docs/audits/0.40/dead-code-and-deps.md. List exact removals as proposed Phase 2 tasks, grouped so each lands alone.

## Log
- 2026-10-17T19:39:23.145Z: [codex] Audit in progress on dispatch/task-104, report only. Early handoff for task-107/109: Knip 6.41.0 finds no unused ui/package.json deps; 30 unused exports mostly need only export removed, generated contract types and mobile-shared pacing/reveal/cadence must not be deleted from scanner output. cargo-deny reports 10 vulnerability diagnostics (rustls + both zip versions), 9 unmaintained; Bun audit reports KaTeX and source-map-js. Report will separate exposure, exact cleanups, and independent followups.
- 2026-10-17T19:43:23.182Z: [codex] Handoff for epic-12/task-109: review docs/audits/0.40/dead-code-and-deps.md, then file the 18 independent Phase 2 proposals after George approves. Prioritize rustls patch and ZIP ownership paths (direct ACP, Tauri updater, sherpa build separately); Bun alerts include a KaTeX copy inside Mermaid's classic bundle, so lockfile overrides alone do not prove it fixed. Coordinate 22 private value exports + 14 private types with task-107 and three stale phase sentences with task-108; retain mobile-shared exports, generated contract, supported platform gates, scripts/packaging/fixtures. All 10 suppression sites accounted for (one redundant). No confirmed unused direct Cargo/npm dependency; machete's tauri-build result is false positive, nightly udeps passes. Enable gates only after approved fixes and explicit upstream exceptions.
- 2026-10-17T19:43:23.473Z: [codex] Finished the read-only audit and committed only its report as f7de57ea on dispatch/task-104. make check, UI build, nightly udeps and all contract validation commands pass; advisory/license scan failures are documented findings. Worktree is clean; ready for human review, not completed.
- 2026-10-17T19:43:38.450Z: Landed on release/0.40.0
