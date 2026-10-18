---
id: task-174
title: "P2-33d (C): Window split, move-only: Teammate.tsx"
column: in-progress
position: 63
description: |-
  Phase 2 group C, plan row 33 (fourth landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

  Move-only: split `ui/src/components/Teammate.tsx` (about 1,800–2,000 lines) by concern into sibling files, re-exporting everything from `components/Teammate.tsx` so no importer changes. Leave the seen-record copy at `Teammate.tsx:1814-1839` where P2-49 (task-158) can find it, moved or not, and say where in a note.

  End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - restructure
  - window
relatedFiles:
  - ui/src/components/Teammate.tsx
parentId: epic-13
createdAt: "2026-10-17T22:45:36.391Z"
contract:
  status: in_progress
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:32:54.141Z"
    reworkCount: 0
updatedAt: "2026-10-18T00:32:54.141Z"
dependsOn:
  - task-122
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:35.631Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:32:29.166Z"
assignee: claude
---

## Description
Phase 2 group C, plan row 33 (fourth landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: split `ui/src/components/Teammate.tsx` (about 1,800–2,000 lines) by concern into sibling files, re-exporting everything from `components/Teammate.tsx` so no importer changes. Leave the seen-record copy at `Teammate.tsx:1814-1839` where P2-49 (task-158) can find it, moved or not, and say where in a note.

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:38:50.805Z: [claude] Done on dispatch/task-174 (40129382), move-only: Teammate.tsx 1849→686 lines (Teammate, VoiceRow, HarnessRows, CollaborationRows, FolderRows/foldersNote). New in components/teammate/: rows.tsx (SwitchRow, FoldRow, RowText, InfoKey, NESTED, the three *_ABOUT strings), ComputerRows.tsx (with SliderRow, LimitRow, AddMount), GrantRows.tsx (McpRows, SkillRows), ToolLedger.tsx, Threads.tsx. Teammate.tsx re-exports BACKGROUND_ABOUT, COMPUTER_ABOUT, MACHINE_ABOUT and SwitchRow, so no importer changed. Only change beyond the move: three doc comments that sat apart from their functions now sit on them. For P2-49 (task-158): the seen-record copy (THREAD_SEEN_KEY, loadThreadSeen, saveThreadSeen) is now in ui/src/components/teammate/Threads.tsx: the key at line 8, the helpers at lines 115-137. Knip is unchanged from baseline. make ui-check and make precheck pass; nothing left unwired.
