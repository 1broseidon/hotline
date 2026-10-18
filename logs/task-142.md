---
id: task-142
title: "P2-33a (C): Window split, move-only: App() → useNavigation, useSeen, useWindowHotkeys"
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:57.498Z"
relatedFiles:
  - ui/src/App.tsx
  - ui/src/useNavigation.ts
  - ui/src/useSeen.ts
  - ui/src/useWindowHotkeys.ts
updatedAt: "2026-10-18T00:26:57.992Z"
contract:
  status: delivered
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:17:09.705Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T00:26:16.445Z"
    duration: 547
dependsOn:
  - task-122
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:02.018Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:15:18.984Z"
assignee: claude
completedAt: "2026-10-18T00:26:57.992Z"
---

## Description
Phase 2 group C, plan row 33 (first landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: lift `useNavigation`, `useSeen` and `useWindowHotkeys` out of `App()` in `ui/src/App.tsx` into their own files under `ui/src/`, behaviour unchanged. No dependency-list triage here (that is P2-48, task-157, after the split: C8). The three other window splits are their own siblings: task-172 (Settings), task-173 (Transcript), task-174 (Teammate).

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:26:20.678Z: [claude] Done on dispatch/task-142, as three commits in the audit's order: 4b143bf0 useNavigation (selectedId and persistence, pane, settingsSection, the per-teammate spaces for dock and inspector, openOnDesk and arriving, select, closePane, togglePane, newTeammate), eb0022b7 useSeen(selectedId, latest), and 24af4a59 useWindowHotkeys (seatOf, the keydown chords, listenMenu, the global hotkeys and registrar sync, and takeChord). Behaviour is unchanged and every dependency list is kept verbatim for P2-48/task-157 to triage. The hooks now live in ui/src/useNavigation.ts, useSeen.ts and useWindowHotkeys.ts. The one non-verbatim edit: the selectedId effect was split in two, so saveSelected lives in useNavigation and setFocusSchedules(false) stays in App. Both have the same deps. App's perDesk wrapper is gone; both files call deskKey directly. Left in App: toggleInspector, openDock/closeDock, removeTeammate, the roster-removal effect, the contextmenu suppress and the layout. useWindowHotkeys takes 26 named inputs, which is a candidate for slimming once the deps are triaged. App.tsx is 967→680 lines. make ui-check passes and make precheck passes all 7 legs.
- 2026-10-18T00:26:55.864Z: Landed on release/0.40.0
