---
id: task-218
title: "Slice 4: pair threads on the one builder and loop"
column: todo
position: 14
priority: medium
tags:
  - threads
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
parentId: epic-20
createdAt: "2026-10-19T19:59:31.591Z"
---

## Description
ADR 0008 §6 slice 4: peer_session builds through Room::thread_agent and drives through Threads::drive. Removes peer_session's own builder, the pair's use of runner::drive and the stale comment in Policy::of. Peer exchange harness tests pass unchanged.
