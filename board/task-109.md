---
id: task-109
title: "Audit synthesis: one ordered Phase 2 plan from the six 0.40 reports"
column: todo
position: 7
priority: high
tags:
  - audit
  - synthesis
relatedFiles:
  - docs/audits/0.40/README.md
  - docs/audits/0.40/structure.md
  - docs/audits/0.40/dead-code-and-deps.md
  - docs/audits/0.40/errors.md
  - docs/audits/0.40/lints.md
  - docs/audits/0.40/window.md
  - docs/audits/0.40/tests-and-docs.md
parentId: epic-12
dependsOn:
  - task-103
  - task-104
  - task-105
  - task-106
  - task-107
  - task-108
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/audits/0.40/README.md
      description: Index and ordered Phase 2 plan
  validation:
    commands:
      - for r in structure dead-code-and-deps errors lints window tests-and-docs; do grep -q "$r.md" docs/audits/0.40/README.md || exit 1; done
      - grep -q '^## Reports' docs/audits/0.40/README.md && grep -q '^## Phase 2 order' docs/audits/0.40/README.md && grep -q '^## Contradictions' docs/audits/0.40/README.md && grep -q '^## Standards' docs/audits/0.40/README.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/README.md')"
  constraints:
    - Commit only docs/audits/0.40/README.md
    - "Sections: '## Reports', '## Phase 2 order', '## Contradictions', '## Standards'"
  metrics:
    readyAt: "2026-10-17T19:29:38.062Z"
createdAt: "2026-10-17T19:29:38.067Z"
updatedAt: "2026-10-17T19:54:18.874Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T19:54:18.750Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 1 ends with one document George can approve from: docs/audits/0.40/README.md.

- Link and summarise each of the six reports in a line or two.
- Merge their proposed Phase 2 tasks into one ordered list. Dedupe overlaps: dead-code removals named by structure, lints and window; allow() reviews named by both dead-code and lints; result_large_err named by both errors and lints.
- Order the list so the steps don't fight each other. Delete dead code before moving files. Make move-only splits before lint fixes in the same files, so blame survives and fixes don't conflict. Turn on the [lints] table last, once its fixes have landed. Add tools to make check only after the code passes them.
- Put the dependency advisories from dead-code-and-deps.md (F1 ZIP extraction RUSTSEC-2026-0367/0368/0369 via driver/acp/install.rs; F2 rustls 0.23.45 → 0.23.46; F3 the npm advisories) at the top of the Phase 2 order as security fixes, ahead of the quality work. F1 changes what an ACP install can write, so mark it as a capability change under docs/security.md.
- For each Phase 2 task, give the source report, the files it touches, what it collides with, and its size.
- Flag every contradiction between reports and recommend a resolution.
- List what docs/standards.md should say, drawn from the reports' conventions: module layout, test layout, file budget, error and panic policy, and lint policy.
- Read-only: don't edit the six reports, and change no code.

## Log
- 2026-10-17T19:54:17.771Z: [claude] Lead: tests-and-docs.md found docs/security.md cites four proof tests that don't exist. Rank that with the advisories, in the security group at the top of the Phase 2 order. Each missing proof means either a capability with no proof, or a stale index; say which for each.
