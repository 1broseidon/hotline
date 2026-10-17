---
id: task-107
title: "Audit: the window — dead exports, big components, lint setup"
column: todo
position: 5
priority: high
tags:
  - audit
  - ui
parentId: epic-12
createdAt: "2026-10-17T19:28:05.232Z"
contract:
  status: ready
  deliverables:
    - type: docs
      path: docs/audits/0.40/window.md
      description: Window (ui/) audit report
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/window.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/window.md && grep -q '^## Leave alone' docs/audits/0.40/window.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/window.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.(ts|tsx):[0-9]+' docs/audits/0.40/window.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/window.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/window.md; run knip and any linter ad hoc, add none to ui/package.json"
    - "Report has exactly these sections: '## Findings' (ranked severity x effort, each with file:line evidence), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - Do not commit ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
updatedAt: "2026-10-17T19:29:26.521Z"
relatedFiles:
  - docs/audits/0.40/window.md
  - ui/src
  - ui/tests
  - ui/package.json
  - ui/tsconfig.json
  - ui/src/generated/contract.ts
---

## Description
Audit: the window (ui/).

- Dead exports and files (npx knip, ad hoc).
- The largest components (Settings.tsx, Transcript.tsx, Teammate.tsx, call.ts, App.tsx): proposed splits along real seams.
- Repeated logic and hand-rolled helpers that duplicate each other, any-casts, effect hygiene (missing deps, effects doing derivation).
- tsconfig strictness, and whether to adopt a linter (Biome or ESLint) and its rule set.
- Test layout in ui/tests.
- generated/contract.ts: how it's produced, and the truncation hazard noted in the repo.
- Report: docs/audits/0.40/window.md.
