---
id: task-119
title: "P2-7 (A): Ship a Mermaid bundle with KaTeX fixed"
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
updatedAt: "2026-10-18T00:25:35.023Z"
contract:
  status: delivered
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:17:03.458Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T00:24:16.761Z"
    duration: 433
dependsOn:
  - task-118
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:39.463Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:14:42.053Z"
    ended: "2026-10-18T00:25:35.023Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T00:25:32.184Z"
---

## Description
Phase 2 group A (Security), plan row 7 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Ship a Mermaid bundle with KaTeX fixed** (GHSA-238p-pmpm-9mq7). Check the minified asset itself, because a lockfile override does not reach the copy embedded in `mermaid.min.js`. Keep classic-script loading, strict mode and plain labels

- Source: dead-code F3, task 6
- Files: `ui/package.json`, `ui/bun.lock`, `ui/src/components/Mermaid.tsx` only if its API moved
- Collides with: 6, 12 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:24:16.279Z: [claude] Mermaid ^11 → ^12.2.0 (first release whose bundle has KaTeX ≥0.18.2; 11.17.2 is the last 11.x and still embeds 0.16.47). Checked the asset itself: node_modules mermaid.min.js and the built dist/assets/mermaid.min-*.js both contain KaTeX's version constant "0.18.11" and no 0.16.47; bun audit is clean (no vulnerabilities). Still a classic script that sets globalThis.mermaid. The only API change: 12 deprecates flowchart.htmlLabels in favour of the root htmlLabels (which takes precedence), so Mermaid.tsx drops the redundant key and keeps strict mode and htmlLabels:false. Rendered flowchart, sequence and class diagrams with our config in a macOS WKWebView: no script or foreignObject, and bad source is rejected. Only $$math$$ labels produce a foreignObject, as 11.17.2 did, and the SVG is still shown as an inert <img>. Not checked on WebKitGTK (Linux). Lockfile: mermaid 12 brings in elkjs, @mermaid-js/parser 2, chevrotain 13 and katex 0.18.11 (commander 15). source-map-js stays at 1.2.2, so row 12 resolves a lockfile conflict by keeping both. The lazy Mermaid script grew from 3.57 MB to 5.49 MB uncompressed; it is still loaded only when a diagram is on screen. make ui-check and make precheck pass (7/7). Commit 035d0ead on dispatch/task-119.
- 2026-10-18T00:25:31.217Z: Landed on release/0.40.0
