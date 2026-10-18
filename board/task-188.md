---
id: task-188
title: Say 'Hotline Agent unavailable' when no provider is connected
column: todo
position: 14
priority: high
tags:
  - providers
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:57.376Z"
relatedFiles:
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/failure.rs
  - ui/src/components/settings
updatedAt: "2026-10-18T23:53:19.400Z"
---

## Description
Slice of task-97, item 1. When no provider is connected, Hotline Agent teammates should say why in a few words ('Hotline Agent unavailable: connect a provider'), with a link to Settings > Providers, not fail on the first turn. The desk.status view and the reachability row stay in task-97.

Split from task-97, which keeps the rest and stays deferred.
