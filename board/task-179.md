---
id: task-179
title: A dropdown's own tooltip covers the first item of its open menu
column: in-progress
position: 1
priority: medium
tags:
  - ui
  - qa
createdAt: "2026-10-18T21:49:01.853Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T21:49:38.356Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T21:49:25.088Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 95396
    at: "2026-10-18T21:49:31.180Z"
assignee: claude
---

## Description
Found in 0.40 QA (2026-10-18). In Settings > Providers, opening the Images or Live calls dropdown shows the trigger's tooltip ('Automatic', 'Off') on top of the menu's first option, hiding it. Hide a trigger's tooltip while its menu is open, in the shared dropdown/combo component, so every dropdown gets it. UI test: with the menu open, no tooltip is shown.

## Log
- 2026-10-18T21:50:16.175Z: [claude] Picker and MenuButton (ui/src/ui/Menu.tsx) drop their title while the menu is open, so every dropdown's tooltip stops covering the first item; ui/tests/menu.test.tsx proves it for both. Committed 94fa2ade on dispatch/task-179. UI typecheck, lint and all 430 UI tests pass.
