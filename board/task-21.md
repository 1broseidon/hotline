---
id: task-21
title: "Startup: open the window before the core finishes settling"
column: todo
position: 23
priority: medium
tags:
  - improvement
  - bro-238
createdAt: "2026-10-11T21:05:31.276Z"
description: |-
  Everything runs on the main thread before the window is built (`hotline-app/src/lib.rs` \~331-361): `block_on(Desk::open)`, `block_on(remote.restore())` (keychain, possible cert generation), `Door::bind`, `block_on(desks.start_all())` (paired desks one by one, `desks.rs` \~82-91).

  `Room` construction runs `settle()` synchronously (`session/lifecycle.rs` \~192-223): loads every tape and pair, compacts each (rewrites whenever the fold differs, which is almost always for an active tape), then the search indexer usually does a full reindex because `index_event` doesn't stamp for non-message kinds (`store/search.rs` \~744-746). A schema upgrade rebuilds the index inside the constructor.

  Fix: bind the door and build the window first; run settle/compaction/reindex in the background (compact only above a duplicate ratio; stamp after every append); restore remote after bind; start paired desks concurrently. The window's `wire.ts` already retries.

  Window side: persist the local desk's roster so the rail paints on first frame (`ui/src/App.tsx` \~674-689), and lazy-load react-markdown after first paint.

  Migrated from Linear BRO-238: https://linear.app/1broseidon/issue/BRO-238/startup-open-the-window-before-the-core-finishes-settling (2026-10-11).
updatedAt: "2026-10-11T21:05:31.573Z"
---
