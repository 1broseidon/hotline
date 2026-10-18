---
id: task-172
title: "P2-33b (C): Window split, move-only: Settings.tsx → components/settings/"
column: todo
position: 61
description: |-
  Phase 2 group C, plan row 33 (second landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

  Move-only: split `ui/src/components/Settings.tsx` into `ui/src/components/settings/*.tsx` by section, and re-export everything from `components/Settings.tsx` so no importer changes. No behaviour change, no fixes (those are P2-47/48/49 after).

  End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - restructure
  - window
relatedFiles:
  - ui/src/components/Settings.tsx
  - ui/src/components/settings/
parentId: epic-13
createdAt: "2026-10-17T22:45:35.818Z"
contract:
  status: ready
  validation:
    commands:
      - test -d ui/src/components/settings
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
updatedAt: "2026-10-18T00:15:33.499Z"
dependsOn:
  - task-122
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:33.366Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Phase 2 group C, plan row 33 (second landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: split `ui/src/components/Settings.tsx` into `ui/src/components/settings/*.tsx` by section, and re-export everything from `components/Settings.tsx` so no importer changes. No behaviour change, no fixes (those are P2-47/48/49 after).

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
