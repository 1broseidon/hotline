---
id: task-118
title: "P2-6 (A): Patch source-map-js"
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:45.054Z"
relatedFiles:
  - ui/bun.lock
  - ui/package.json
updatedAt: "2026-10-17T23:35:07.168Z"
contract:
  status: delivered
  validation:
    commands:
      - "! grep -qE 'source-map-js@1\\.([01]\\.[0-9]+|2\\.[01])\"' ui/bun.lock"
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T23:11:29.470Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T23:17:12.204Z"
    duration: 343
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T23:10:41.785Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T23:11:03.081Z"
assignee: claude
completedAt: "2026-10-17T23:35:07.168Z"
---

## Description
Phase 2 group A (Security), plan row 6 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Patch source-map-js** to `>=1.2.2` within the parent ranges (GHSA-68fv-2mgg-jv7q)

- Source: dead-code F3, task 5
- Files: `ui/bun.lock`
- Collides with: 7, 12 (lockfile)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-17T23:17:11.615Z: [claude] ui/bun.lock now resolves source-map-js 1.2.2 (one-line lockfile edit, npm integrity, no package.json change; postcss and @tailwindcss/node both accept ^1.2.1). bun install --frozen-lockfile, bun audit (only KaTeX low left, that is row 7), bun run build, make ui-check and make precheck all pass. Committed 44805428 on dispatch/task-118. For rows 7 and 12: the only lockfile change is the source-map-js package line, so a conflict there resolves by keeping 1.2.2.
- 2026-10-17T23:35:04.024Z: Landed on release/0.40.0
