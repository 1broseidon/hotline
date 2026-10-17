---
id: task-108
title: "Audit: tests (slow, flaky, duplicated) and docs drift"
column: review
position: 6
priority: high
tags:
  - audit
  - tests
  - docs
parentId: epic-12
createdAt: "2026-10-17T19:28:05.760Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-17T19:42:16.112Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T19:53:46.910Z"
    duration: 691
updatedAt: "2026-10-17T19:54:03.114Z"
relatedFiles:
  - docs/audits/0.40/tests-and-docs.md
  - crates/hotline-core/tests
  - docs
  - docs-site/src
  - README.md
  - AGENTS.md
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T19:41:54.589Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T19:41:56.593Z"
    ended: "2026-10-17T19:53:54.282Z"
    outcome: review
assignee: claude
---

## Description
Audit: tests and docs.

- Tests: slowest tests (cargo test -- -Z unstable-options --report-time, or timing runs), flaky patterns (shared temp paths, clock-based names, fixed ports, sleeps, network-dependent tests: two were fixed 2026-10-17, so find their siblings), duplicated harness code across tests/*.rs and in-file modules, and tests that test nothing.
- Docs: docs/*.md and docs-site against the code (commands, settings names, file paths, ADR status), README links, and stale release notes. Doc drift is a bug: list every mismatch.
- Report: docs/audits/0.40/tests-and-docs.md.

## Log
- 2026-10-17T19:53:50.279Z: [claude] Report at docs/audits/0.40/tests-and-docs.md on dispatch/task-108 (5f77588e): 15 test findings, 11 doc findings, 18 phase-2 tasks. For the next tasks: (1) security.md cites 4 tests that don't exist (:330 :683 :703 :1426), so fix those first; (2) the remote test Harness connects through the host's first real address (remote/tests.rs:115), the sibling of 1859ec74, and the 15s ANSWER from a1cd577d is private to sealed_network while relay/admission tests still wait 1-3s; (3) nothing uses a paused tokio clock, and the 30s viewer-token sleep plus the 10s xai deadline are ~40s of a 98s suite; (4) a new worktree needs .speech-engine (copy from a sibling worktree and run scripts/fetch-speech-engine) before cargo test builds; (5) the full suite passed and left contract.ts unchanged.
- 2026-10-17T19:54:03.114Z: Landed on release/0.40.0
