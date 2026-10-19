---
id: task-222
title: 0.41 lands together, and Pork Chop's QA on the Linux QA computer before George's
column: todo
position: 18
priority: high
tags:
  - qa
  - "0.41"
assignee: claude
relatedFiles:
  - docs
  - CHANGELOG.md
parentId: epic-20
createdAt: "2026-10-19T19:59:33.586Z"
dependsOn:
  - task-219
  - task-218
  - task-221
  - task-226
  - task-202
  - task-205
updatedAt: "2026-10-19T20:03:15.032Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: one harness proving epic-20's pieces together over the wire
    - type: docs
      path: CHANGELOG.md
      description: Unreleased says what 0.41 changed
  validation:
    commands:
      - make check
      - make precheck
      - make verify
  metrics:
    readyAt: "2026-10-19T20:03:15.032Z"
---

## Description
The epic's wiring and proof, last. After every 0.41 child (this epic's and epic-17's task-202 and task-205): first a headless harness in crates/hotline-core/tests/ (threads.rs or a new one) that drives the real core over the wire through the whole of epic-20 together, against its description of done: one teammate on each driver; post_to_chat from a quiet job, a work thread, a pair and an ACP teammate, each landing in the chat stamped with where it came from; a loud DM turn and a subagent refused; a file from a work thread landing in the chat; a recurring job on its own thread with its running note, a skipped overlap and a final post on cancel; a subagent reaching back with search_thread under the home thread's lease. Fix any seam between the slices it finds. Then make check, make precheck and CI (ci.yml and workspace-isolation.yml dispatched on release/0.41.0) green; docs (sessions.md, threads.md, security.md, design.md) and the CHANGELOG's Unreleased section say what changed. Then Pork Chop runs the desk from release/0.41.0 on the Linux QA computer (its prepared workspace, xAI sign-in) and exercises: post_to_chat from a quiet job, a work thread and an ACP teammate; a file from a work thread landing in the chat with its stamp; a recurring job (its thread, note, a skipped overlap and the suggested interval, a post on significance, cancel with a final post); a one-off job and a plain reminder; the Jobs group rows; a subagent reaching back with search_thread; the team lead prompt, badge and single-teammate auto-lead. Screenshots of anything wrong; fix or file before handing George the QA checklist.

## Log
- 2026-10-19T20:00:42.947Z: [claude] Split of work: the dispatched agent does the land-together part (check, precheck, CI, docs, CHANGELOG) and writes the QA checklist into epic-20's log; the hands-on QA on the Linux QA computer is Pork Chop's, from the watch loop, since only Pork Chop drives that computer. Hand-off to George waits for that QA.
