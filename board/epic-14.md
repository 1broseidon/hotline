---
id: epic-14
title: "0.40 easy wins: the small fixes from the backlog triage"
type: epic
column: todo
position: 1
priority: high
tags:
  - "0.40"
  - easy-wins
  - triage
createdAt: "2026-10-18T23:40:44.455Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T23:51:14.157Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T23:51:14.029Z"
  until: done
  role: lead
assignee: claude
---

## Description
The next wave for 0.40: small, low-risk fixes found by the 2026-10-18 backlog triage (four reviewers checked all 44 backlog items against the post-restructure code at release/0.40.0 78255a4f). Each child is one slice. Where a backlog item was only partly in scope, the slice is a new task, and the original keeps the rest and stays deferred.

Rules: each child is one landable change behind `make precheck` (the hooks are on), no new feature surface, and follows docs/standards.md. QA at the end on the Linux QA desktop with George's xAI sign-in, as for the 2026-10-18 pass. All land on release/0.40.0.
