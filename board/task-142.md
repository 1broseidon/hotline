---
id: task-142
title: "P2-33a (C): Window split, move-only: App() → useNavigation, useSeen, useWindowHotkeys"
column: todo
position: 31
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:57.498Z"
relatedFiles:
  - ui/src/App.tsx
  - ui/src/useNavigation.ts
  - ui/src/useSeen.ts
  - ui/src/useWindowHotkeys.ts
updatedAt: "2026-10-17T22:45:13.877Z"
---

## Description
Phase 2 group C, plan row 33 (first landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: lift `useNavigation`, `useSeen` and `useWindowHotkeys` out of `App()` in `ui/src/App.tsx` into their own files under `ui/src/`, behaviour unchanged. No dependency-list triage here (that is P2-48, task-157, after the split: C8). The three other window splits are their own siblings: task-172 (Settings), task-173 (Transcript), task-174 (Teammate).

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
