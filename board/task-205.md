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
updatedAt: "2026-10-19T20:02:51.009Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: ui/tests/team-lead.test.tsx
      description: badge, menu, pick-a-lead prompt and onboarding line, all from the roster
    - type: docs
      path: CHANGELOG.md
      description: a line under Unreleased on the team lead
  validation:
    commands:
      - test -f ui/tests/team-lead.test.tsx
      - cd ui && bun test tests/team-lead.test.tsx
      - grep -qi 'lead' CHANGELOG.md
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make verify
  constraints:
    - The window never sets a lead on its own; everything reflects the roster's lead
    - No usage counter added for the suggestion
    - No core or contract changes beyond what task-202 landed; if one is needed, say so in a note
  metrics:
    readyAt: "2026-10-19T20:02:38.203Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:02:50.866Z"
  until: done
  tries: 0
---

## Description
The last piece of epic-17's 0.41 half, and the one that wires it together: the window drives task-202's lead commands and reflects the roster's lead, nothing held only in the window.

Make team lead in a teammate's right-click menu (Rail.tsx) and on its card (Teammate.tsx), and Remove as lead on the lead. A small badge on the lead in the team list (tooltip: "Team lead"). With two or more teammates and no lead, a small prompt asks the person to pick one, suggesting their oldest teammate (or the most-used, if the roster already says so; do not add a usage counter for this); Not now dismisses it until the next desk call or the next launch. Deleting the lead brings the prompt back when two or more remain. Onboarding (Welcome.tsx / NewTeammate.tsx) says the first teammate leads the team, in one line. No fake UI: the badge, menu label and prompt all read the roster's lead; the core already sets it for a one-teammate desk, so the window never sets a lead on its own.

Proof: a ui test (ui/tests/team-lead.test.tsx, in the style of the existing DOM tests) that feeds rosters and checks: the badge follows the roster's lead; the menu sends the lead command and says Remove as lead on the lead; the prompt shows only with two or more and no lead, names the suggestion, and Not now hides it; onboarding's line is present. Then make verify, which runs task-202's lead harness against the merged core, so the whole 0.41 half is proved together. A CHANGELOG line under Unreleased: what a team lead is and how to pick one.
