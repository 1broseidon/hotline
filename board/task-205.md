---
id: task-205
title: "Desktop: Make team lead, the lead's badge, and the pick-a-lead prompt"
column: todo
position: 5
priority: medium
tags:
  - ui
assignee: claude
relatedFiles:
  - ui/src/components/Rail.tsx
  - ui/src/components/Teammate.tsx
  - ui/src/components/Welcome.tsx
  - ui/src/components/NewTeammate.tsx
  - ui/src/App.tsx
  - ui/src/wire.ts
  - ui/tests
  - CHANGELOG.md
parentId: epic-17
createdAt: "2026-10-19T13:59:39.118Z"
dependsOn:
  - task-202
updatedAt: "2026-10-19T20:02:33.787Z"
---

## Description
The last piece of epic-17's 0.41 half, and the one that wires it together: the window drives task-202's lead commands and reflects the roster's lead, nothing held only in the window.

Make team lead in a teammate's right-click menu (Rail.tsx) and on its card (Teammate.tsx), and Remove as lead on the lead. A small badge on the lead in the team list (tooltip: "Team lead"). With two or more teammates and no lead, a small prompt asks the person to pick one, suggesting their oldest teammate (or the most-used, if the roster already says so; do not add a usage counter for this); Not now dismisses it until the next desk call or the next launch. Deleting the lead brings the prompt back when two or more remain. Onboarding (Welcome.tsx / NewTeammate.tsx) says the first teammate leads the team, in one line. No fake UI: the badge, menu label and prompt all read the roster's lead; the core already sets it for a one-teammate desk, so the window never sets a lead on its own.

Proof: a ui test (ui/tests/team-lead.test.tsx, in the style of the existing DOM tests) that feeds rosters and checks: the badge follows the roster's lead; the menu sends the lead command and says Remove as lead on the lead; the prompt shows only with two or more and no lead, names the suggestion, and Not now hides it; onboarding's line is present. Then make verify, which runs task-202's lead harness against the merged core, so the whole 0.41 half is proved together. A CHANGELOG line under Unreleased: what a team lead is and how to pick one.
