---
id: task-202
title: "Core + contract: the team lead role, set, moved and kept"
column: todo
position: 2
priority: high
tags:
  - core
  - contract
assignee: claude
relatedFiles:
  - crates/hotline-contract
  - crates/hotline-core/src/room.rs
  - crates/hotline-core/src/session.rs
parentId: epic-17
createdAt: "2026-10-19T13:59:37.690Z"
---

## Description
The room holds one lead (a persona id) and sends it on the roster; a client that predates it ignores it. Commands to make a teammate lead and to clear it, under the owner checks other room commands use. Rules: onboarding's first teammate becomes lead; a desk with exactly one teammate and no lead sets it (on upgrade too); removing the lead clears it; nothing ever picks among two or more. The lead may hand work to teammates on a desk call without first-contact approval (security.md applies: default, grant source, enforcement points, tests, residual risk). contract.ts regenerated whole.
