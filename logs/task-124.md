---
id: task-124
title: "P2-12 (B): Deduplicate `@tauri-apps/api`"
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:48.155Z"
relatedFiles:
  - ui/bun.lock
  - ui/package.json
updatedAt: "2026-10-18T00:34:38.564Z"
contract:
  status: delivered
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:26:43.455Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T00:32:02.332Z"
    duration: 319
dependsOn:
  - task-119
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:41.702Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:26:27.218Z"
    ended: "2026-10-18T00:34:38.564Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T00:34:35.704Z"
---

## Description
Phase 2 group B (Delete dead code), plan row 12 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Deduplicate `@tauri-apps/api`** onto the tested direct pin

- Source: dead-code F9, task 13
- Files: `ui/bun.lock:687`
- Collides with: 6, 7
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:32:01.902Z: [claude] Deleted the nested "@tauri-apps/plugin-global-shortcut/@tauri-apps/api" 2.12.1 entry from ui/bun.lock (its ^2.8.0 range now resolves to the direct 2.11.1 pin). No package.json, plugin or override change. bun install --frozen-lockfile, a fresh install from scratch, and a plain bun install all leave the lockfile alone ("no changes"), and there is no nested node_modules under the plugin. make ui-check (420 tests, hotkeys included), vite build and make precheck (7/7) pass. Desktop shortcuts were not checked by hand in the running app. Nothing left unwired. Rows 6/7 lockfile collisions: only this one line changes, so it merges cleanly over task-119. Commit b9387a8d on dispatch/task-124.
- 2026-10-18T00:34:34.480Z: Landed on release/0.40.0
