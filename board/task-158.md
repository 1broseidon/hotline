---
id: task-158
title: "P2-49 (D): Window plumbing"
column: todo
position: 47
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:05.837Z"
relatedFiles:
  - ui/src/storage.ts
  - ui/src/App.tsx
  - ui/src/components/Teammate.tsx
  - ui/tests/dom.ts
  - ui/tests/
  - ui/tsconfig.json
updatedAt: "2026-10-17T22:47:19.481Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 49 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Window plumbing.** `storage.ts` for the two seen-record copies. `tests/dom.ts` for the 22 hand-built happy-dom windows. Add `tests` to `tsconfig` `include`, plus `noImplicitReturns` and `noPropertyAccessFromIndexSignature`, fixing what surfaces

- Source: window F5, F7, F9, tasks 3, 4, 6
- Files: `ui/src/App.tsx:912-935`, `Teammate.tsx:1814-1839`, `ui/tests/*`, `ui/tsconfig.json`
- Collides with: 33
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
