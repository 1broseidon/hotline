---
id: task-142
title: "P2-33 (C): Window splits, move-only, each re-exported from its old path"
column: todo
position: 31
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:57.498Z"
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 33 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Window splits, move-only, each re-exported from its old path.** First `App()` → `useNavigation`, `useSeen`, `useWindowHotkeys`. Then `Settings.tsx` → `components/settings/`, `Transcript.tsx` (folds to `transcriptFolds.ts` first), and `Teammate.tsx`. One landing each

- Source: window F2, F3, tasks 9–12
- Files: `ui/src/App.tsx`, `components/{Settings,Transcript,Teammate}.tsx`
- Collides with: 10 (land first); 48, 49 (fixes follow)
- Size: L

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
