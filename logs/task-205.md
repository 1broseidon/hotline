---
id: task-205
title: "Desktop: Make team lead, the lead's badge, and the pick-a-lead prompt"
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
updatedAt: "2026-10-19T21:33:25.063Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T20:54:39.606Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T21:30:59.303Z"
    duration: 2180
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:02:50.866Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:54:27.843Z"
    ended: "2026-10-19T21:31:17.024Z"
    outcome: review
completedAt: "2026-10-19T21:33:25.063Z"
---

## Description
The last piece of epic-17's 0.41 half, and the one that wires it together: the window drives task-202's lead commands and reflects the roster's lead, nothing held only in the window.

Make team lead in a teammate's right-click menu (Rail.tsx) and on its card (Teammate.tsx), and Remove as lead on the lead. A small badge on the lead in the team list (tooltip: "Team lead"). With two or more teammates and no lead, a small prompt asks the person to pick one, suggesting their oldest teammate (or the most-used, if the roster already says so; do not add a usage counter for this); Not now dismisses it until the next desk call or the next launch. Deleting the lead brings the prompt back when two or more remain. Onboarding (Welcome.tsx / NewTeammate.tsx) says the first teammate leads the team, in one line. No fake UI: the badge, menu label and prompt all read the roster's lead; the core already sets it for a one-teammate desk, so the window never sets a lead on its own.

Proof: a ui test (ui/tests/team-lead.test.tsx, in the style of the existing DOM tests) that feeds rosters and checks: the badge follows the roster's lead; the menu sends the lead command and says Remove as lead on the lead; the prompt shows only with two or more and no lead, names the suggestion, and Not now hides it; onboarding's line is present. Then make verify, which runs task-202's lead harness against the merged core, so the whole 0.41 half is proved together. A CHANGELOG line under Unreleased: what a team lead is and how to pick one.

## Log
- 2026-10-19T20:55:16.842Z: [claude] Lead: task-202 has landed on release/0.41.0, so this child can run. Three children so far ended their turn while make check was still running in the background and left work uncommitted. Here: never use run_in_background. Run each check as a foreground Bash call with timeout 600000: `cargo test -p hotline-contract` and `-p hotline-core` unfiltered (contract.ts written whole), the ui test, `make check`, `make verify`. Commit. Then merge release/0.41.0 into the branch, re-run `make check` in the foreground, and end only when it passes and `git status` is clean.
- 2026-10-19T21:31:05.596Z: [claude] Done on dispatch/task-205. Rail right-click (rows and pinned faces, any desk via wireFor) and the teammate pane's profile card ("Team lead · <name>" row) send persona.lead: Make team lead / Remove as lead, with Remove disabled for the only teammate. A star (title "Team lead", row label ", team lead") marks the lead. LeadPrompt.tsx sits under DeskBand in the main column and shows for 2+ teammates with no lead, suggesting the oldest by persona.createdAt (the roster has no usage count, so none was added). Not now is held per desk in module state until the next launch or desk call; a lead named clears it, so losing the lead asks again. Welcome's first-teammate lead line now says the first teammate leads. Everything is read from roster.lead; the window never sets a lead unasked. Proof: ui/tests/team-lead.test.tsx (6 tests), make check and make verify pass, contract.ts unchanged, CHANGELOG under Unreleased. One core test change: task-202's unit test the_lead_is_set_moved_and_cleared_and_the_roster_rows_carry_it hung about 5 runs in 6. heard_where drops frames it passes over, and the persona.lead answer and the roster rows it moves come from different tasks. Fixed with a heard_each helper in wire/tests/roster.rs (20/20 runs pass); no behaviour change. The pin test above it uses the same answer-then-row pattern and could flake the same way. Not wired: desk calls do not reach the lead yet (epic's later work), so the prompt doesn't claim they do. The desk-call reset of Not now is wired (deskCalling from App) but not covered by the test.
- 2026-10-19T21:33:24.652Z: Landed on release/0.41.0
