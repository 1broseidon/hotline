---
id: task-104
title: "Audit: dead code and dependencies (Rust and window)"
column: in-progress
position: 2
priority: high
tags:
  - audit
  - dead-code
parentId: epic-12
createdAt: "2026-10-17T19:28:03.692Z"
contract:
  status: in_progress
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
updatedAt: "2026-10-17T19:35:39.904Z"
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
