---
id: task-108
title: "Audit: tests (slow, flaky, duplicated) and docs drift"
column: todo
position: 6
priority: high
tags:
  - audit
  - tests
  - docs
parentId: epic-12
createdAt: "2026-10-17T19:28:05.760Z"
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/audits/0.40/tests-and-docs.md
      description: Tests and docs drift audit report
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/tests-and-docs.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/tests-and-docs.md && grep -q '^## Leave alone' docs/audits/0.40/tests-and-docs.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/tests-and-docs.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.(rs|md):[0-9]+' docs/audits/0.40/tests-and-docs.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/tests-and-docs.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/tests-and-docs.md; list doc mismatches, do not fix them here"
    - "Report has exactly these sections: '## Findings' (ranked severity x effort, each with file:line evidence), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - If a cargo run rewrites ui/src/generated/contract.ts, restore it before committing
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
updatedAt: "2026-10-17T19:29:26.771Z"
relatedFiles:
  - docs/audits/0.40/tests-and-docs.md
  - crates/hotline-core/tests
  - docs
  - docs-site/src
  - README.md
  - AGENTS.md
---

## Description
Audit: tests and docs.

- Tests: slowest tests (cargo test -- -Z unstable-options --report-time, or timing runs), flaky patterns (shared temp paths, clock-based names, fixed ports, sleeps, network-dependent tests: two were fixed 2026-10-17, so find their siblings), duplicated harness code across tests/*.rs and in-file modules, and tests that test nothing.
- Docs: docs/*.md and docs-site against the code (commands, settings names, file paths, ADR status), README links, and stale release notes. Doc drift is a bug: list every mismatch.
- Report: docs/audits/0.40/tests-and-docs.md.
