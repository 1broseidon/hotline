---
id: task-159
title: "P2-50 (D): a11y pass:"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:06.356Z"
relatedFiles:
  - ui/src/components/
  - ui/src/App.tsx
updatedAt: "2026-10-18T03:25:50.891Z"
contract:
  status: delivered
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:14:09.273Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T03:24:37.991Z"
    duration: 629
dependsOn:
  - task-156
  - task-158
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:21.041Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:12:55.940Z"
    ended: "2026-10-18T03:25:50.891Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T03:25:50.481Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 50 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**a11y pass:** the 9 `noAutofocus`, 8 `useSemanticElements` and 8 `noStaticElementInteractions` hits, as separate small UI changes

- Source: window F6, task 14
- Files: the hit sites
- Collides with: 33, 48
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T03:24:43.833Z: [claude] Done on dispatch/task-159 in three commits, one per rule (6da0ef9f, 18e5fe35, 45380a38). All 25 hits are cleared under Biome 2.5.15 with only those three rules on. noAutofocus (9): every site gets focus right after a press opened it (Add a schedule, Add a secret/Replace, New folder, Add key, Add a note, Custom image…, the New teammate pane), so autoFocus stays and each has a biome-ignore naming the press. useSemanticElements (8): Dock's thread page is now <section>, the menu rule is <hr> (.menu-rule gets border:0), Rail pins, Starters and the relay offer are fieldsets (min-width 0), and Chips is now a fieldset of aria-pressed buttons instead of role=radio, because nothing gave it arrow keys. New ui/tests/chips.test.tsx covers this. The Dock and Rail resize handles keep role=separator (ARIA's window splitter) with a biome-ignore. noStaticElementInteractions (8): the Band and Titlebar drag regions and each WindowEdges strip are aria-hidden. AgentSignIn's wrapper is a <section>, and its key-stopping terminal host has a biome-ignore. The ServerFiles and Viewer backdrops and the two bubbles' context menus have biome-ignores that name the keyboard path. For task 53: the a11y rules can go to error. Not fixed: own-message bubbles (said-me) are not focusable and the bubble-actions buttons are tabIndex -1, so the keyboard cannot reach Copy or Reply on your own messages. The non-recommended a11y rules (noNoninteractiveElementInteractions, useAriaPropsSupportedByRole) still fire and are out of scope. make ui-check and make precheck (7/7) pass.
- 2026-10-18T03:25:50.079Z: Landed on release/0.40.0
