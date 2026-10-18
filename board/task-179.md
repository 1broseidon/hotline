---
id: task-179
title: A dropdown's own tooltip covers the first item of its open menu
column: todo
position: 2
priority: medium
tags:
  - ui
  - qa
createdAt: "2026-10-18T21:49:01.853Z"
---

## Description
Found in 0.40 QA (2026-10-18). In Settings > Providers, opening the Images or Live calls dropdown shows the trigger's tooltip ('Automatic', 'Off') on top of the menu's first option, hiding it. Hide a trigger's tooltip while its menu is open, in the shared dropdown/combo component, so every dropdown gets it. UI test: with the menu open, no tooltip is shown.
