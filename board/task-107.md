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
