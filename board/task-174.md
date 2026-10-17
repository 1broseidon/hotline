---
id: task-174
title: "P2-33d (C): Window split, move-only: Teammate.tsx"
column: todo
position: 63
description: |-
  Phase 2 group C, plan row 33 (fourth landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

  Move-only: split `ui/src/components/Teammate.tsx` (about 1,800–2,000 lines) by concern into sibling files, re-exporting everything from `components/Teammate.tsx` so no importer changes. Leave the seen-record copy at `Teammate.tsx:1814-1839` where P2-49 (task-158) can find it, moved or not, and say where in a note.

  End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - restructure
  - window
relatedFiles:
  - ui/src/components/Teammate.tsx
parentId: epic-13
createdAt: "2026-10-17T22:45:36.391Z"
contract:
  status: ready
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
updatedAt: "2026-10-17T22:48:38.570Z"
dependsOn:
  - task-122
---

## Description
Phase 2 group C, plan row 33 (fourth landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: split `ui/src/components/Teammate.tsx` (about 1,800–2,000 lines) by concern into sibling files, re-exporting everything from `components/Teammate.tsx` so no importer changes. Leave the seen-record copy at `Teammate.tsx:1814-1839` where P2-49 (task-158) can find it, moved or not, and say where in a note.

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
