---
id: task-122
title: "P2-10 (B): Window dead code"
column: in-progress
position: 11
priority: high
tags:
  - phase2
  - dead-code
parentId: epic-13
createdAt: "2026-10-17T22:40:47.122Z"
relatedFiles:
  - ui/src/icons.tsx
  - ui/src/voice/call.ts
  - ui/src/useFor.ts
  - ui/src/components/Transcript.tsx
  - ui/src/components/Rail.tsx
  - ui/src/components/Skills.tsx
  - ui/src/wire.ts
updatedAt: "2026-10-17T23:04:07.693Z"
contract:
  status: in_progress
  validation:
    commands:
      - "! git grep -nE '\\b(BookIcon|currentCall|voiceOf)\\b' -- ui/src"
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T23:04:07.693Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T23:03:22.558Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T23:03:34.645Z"
assignee: claude
---

## Description
Phase 2 group B (Delete dead code), plan row 10 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Window dead code.** Delete `BookIcon`, `currentCall` and `voiceOf`, which have no use beyond their definition (C1). Drop `export` from the other value exports in dead-code F7 that have a use inside their own file (`windowTitle` among them), and from the 14 types. Keep `cadence.ts`, `pacing.ts`, `reveal.ts` and `Segment`, which Hotline Mobile copies (C2). Re-run knip

- Source: dead-code F5, F7, tasks 8 and 10; window F4, tasks 1 and 2
- Files: `ui/src/icons.tsx:197`, `voice/call.ts:32,155,160,1029`, `useFor.ts:38-127`, `Transcript.tsx:1310,2034`, `Rail.tsx:508`, `Skills.tsx:15`, `wire.ts:71-310`, and the rest of the list in dead-code F7
- Collides with: 33 (window splits) and 54 (knip gate): land first
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
