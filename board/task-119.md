---
id: task-119
title: "P2-7 (A): Ship a Mermaid bundle with KaTeX fixed"
column: todo
position: 8
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:45.566Z"
relatedFiles:
  - ui/package.json
  - ui/bun.lock
  - ui/src/components/Mermaid.tsx
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-118
---

## Description
Phase 2 group A (Security), plan row 7 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Ship a Mermaid bundle with KaTeX fixed** (GHSA-238p-pmpm-9mq7). Check the minified asset itself, because a lockfile override does not reach the copy embedded in `mermaid.min.js`. Keep classic-script loading, strict mode and plain labels

- Source: dead-code F3, task 6
- Files: `ui/package.json`, `ui/bun.lock`, `ui/src/components/Mermaid.tsx` only if its API moved
- Collides with: 6, 12 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
