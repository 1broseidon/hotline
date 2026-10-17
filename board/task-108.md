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
---

## Description
Audit: tests and docs.

- Tests: slowest tests (cargo test -- -Z unstable-options --report-time, or timing runs), flaky patterns (shared temp paths, clock-based names, fixed ports, sleeps, network-dependent tests: two were fixed 2026-10-17, so find their siblings), duplicated harness code across tests/*.rs and in-file modules, and tests that test nothing.
- Docs: docs/*.md and docs-site against the code (commands, settings names, file paths, ADR status), README links, and stale release notes. Doc drift is a bug: list every mismatch.
- Report: docs/audits/0.40/tests-and-docs.md.
