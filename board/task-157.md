---
id: task-157
title: "P2-48 (D): Window fixes that Biome needs first"
column: todo
position: 46
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:05.313Z"
relatedFiles:
  - ui/src/App.tsx
  - ui/src/components/Settings.tsx
  - ui/src/components/settings/
  - ui/src/components/ConnectProvider.tsx
  - ui/src/components/Welcome.tsx
  - ui/src/icons.tsx
  - ui/src/stepText.ts
  - ui/src/voice/call.ts
  - ui/src/voice/dictation.ts
updatedAt: "2026-10-18T00:15:18.984Z"
contract:
  status: ready
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-142
  - task-172
  - task-173
  - task-174
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:18.843Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 48 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Window fixes that Biome needs first.** Triage the 52 `useExhaustiveDependencies` hits, one file per commit and `App.tsx` first, with a failing test before each real stale closure. Fix `stepText.ts:14` (`biome-ignore` with a reason), the two `useIterableCallbackReturn` and the two `useExponentiationOperator` hits. Give the icon wrapper `aria-hidden` (37 hits)

- Source: window F1, F6, tasks 8, 13, and the fixes in 7
- Files: `ui/src/App.tsx`, `Settings.tsx`, `ConnectProvider.tsx`, `Welcome.tsx`, `icons.tsx`, `stepText.ts`, `voice/call.ts`, `dictation.ts`
- Collides with: 33 (land first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
