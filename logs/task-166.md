---
id: task-166
title: "P2-D-6 (E): `docs/standards.md`"
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:10.035Z"
relatedFiles:
  - docs/standards.md
  - AGENTS.md
updatedAt: "2026-10-18T06:43:56.619Z"
contract:
  status: delivered
  validation:
    commands:
      - test -s docs/standards.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T06:35:43.939Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T06:41:31.751Z"
    duration: 348
dependsOn:
  - task-165
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:26.653Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T06:35:23.419Z"
    ended: "2026-10-18T06:43:56.619Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T06:43:56.210Z"
---

## Description
Phase 2 group E (Docs), plan row D-6 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`docs/standards.md`** from the [Standards](#standards) section below, once George approves it

- Source: this file
- Files: `docs/standards.md` (new)
- Collides with: D-5
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T06:41:38.307Z: [claude] Done on dispatch/task-166, commit 4c0be615. Docs only. New docs/standards.md is the README's Standards section, approved as written (epic-13), in present-tense rules. The window file budget, which was "my proposal" in the README, is stated as a rule: the same numbers, with call.ts's Call class as the exception. AGENTS.md "Code standards" links to it. make precheck: all 8 legs passed. Unwired, for the tasks after this one: two rules are not true of the tree yet. There is no [workspace.lints] or rust-version until P2-52 (task-167) lands, and no shared until(what, cond) until P2-45b (task-175) lands. Those tasks make the doc true. If either one changes the rule, it edits docs/standards.md in the same commit.
- 2026-10-18T06:43:55.811Z: Landed on release/0.40.0
