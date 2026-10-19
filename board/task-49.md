---
id: task-49
title: "Window: render long conversations progressively with stable scroll anchoring"
column: todo
position: 5
priority: medium
tags:
  - performance
  - ui
relatedFiles:
  - ui/src/components/Transcript.tsx
  - ui/src/App.tsx
  - ui/src/index.css
parentId: epic-14
createdAt: "2026-10-12T04:20:52.535Z"
updatedAt: "2026-10-19T00:44:04.356Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: ui/src/components
      description: Mermaid draws only near the viewport; computer frame blobs cached by event id and revoked; scroll anchor holds when older lines arrive
  validation:
    commands:
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:43:58.519Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:44:04.356Z"
assignee: claude
---

## Description
Split from task-22, preserving BRO-242. Transcript renders all blocks on teammate switch. Paint recent rows first, then older rows; choose measured virtualization if needed and preserve scroll position, search jumps and reply links on WebKit. Draw Mermaid near the viewport and cache computer frame blob URLs by event id with cleanup. Verify long tapes, rapid switching, paging older rows and jumping to results; measure time to first usable conversation. Keep this independent of streaming projection work in task-22.

Related Files:
  - ui/src/components/Transcript.tsx
  - ui/src/App.tsx
  - ui/src/index.css

0.40 triage (2026-10-18): Narrowed: the window already opens on the last lines and pages older ones (08718921, wire/phone.rs DESK_TAPE_WINDOW). Remaining: draw Mermaid only near the viewport (IntersectionObserver), cache computer frame blobs, and add a scroll-anchor check when older lines arrive.
