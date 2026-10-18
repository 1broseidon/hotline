---
id: task-173
title: "P2-33c (C): Window split, move-only: Transcript.tsx (folds to transcriptFolds.ts first)"
column: in-progress
position: 62
description: |-
  Phase 2 group C, plan row 33 (third landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

  Move-only: first move the fold logic out of `ui/src/components/Transcript.tsx` into `ui/src/transcriptFolds.ts`, then split the rest of the component by concern, re-exporting everything from `components/Transcript.tsx` so no importer changes. No behaviour change.

  End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - restructure
  - window
relatedFiles:
  - ui/src/components/Transcript.tsx
  - ui/src/transcriptFolds.ts
parentId: epic-13
createdAt: "2026-10-17T22:45:36.110Z"
contract:
  status: in_progress
  validation:
    commands:
      - test -f ui/src/transcriptFolds.ts
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:31:04.589Z"
    reworkCount: 0
updatedAt: "2026-10-18T00:31:04.589Z"
dependsOn:
  - task-122
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:34.492Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:30:07.217Z"
assignee: claude
---

## Description
Phase 2 group C, plan row 33 (third landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: first move the fold logic out of `ui/src/components/Transcript.tsx` into `ui/src/transcriptFolds.ts`, then split the rest of the component by concern, re-exporting everything from `components/Transcript.tsx` so no importer changes. No behaviour change.

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
