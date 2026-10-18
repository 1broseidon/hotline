---
id: task-24
title: "Phone link: compress frames inside the sealed channel and resume from a cursor"
column: backlog
position: 9
priority: medium
tags:
  - improvement
  - bro-245
createdAt: "2026-10-11T21:05:33.780Z"
description: |-
  0.33.4 + the lean OTA trim what the desk sends a phone. Two things remain that need more than an OTA:

  * **Compression.** No compression at any layer; the sealed path is JSON → Noise ChaChaPoly, so it must happen before `sealed::encode` (e.g. a deflate flag bit beside the existing final-chunk flag byte, `remote/sealed.rs` \~55-70). The native transports (Swift `SealedTransport`, Kotlin `Sockets.kt`) unwrap records, so this needs a native build, gated by a hello capability.
  * **Resume from a cursor.** Every reconnect re-subscribes and gets a full snapshot; there is no seq/since. A naive `after: eventId` misses in-place revisions of older events (receipts, reactions, tool status), so it needs record positions or a per-stream sequence number on appends.
  * Also: per-token deltas are one frame each with no coalescing (`session/dm.rs` \~313-328); batch \~50 ms for phone sockets.

  Migrated from Linear BRO-245: https://linear.app/1broseidon/issue/BRO-245/phone-link-compress-frames-inside-the-sealed-channel-and-resume-from-a (2026-10-11).

  ## Status (2026-10-11 review)

  Needs a native phone build too.

  Closed 2026-10-18 (backlog triage): Won't do until something shows the phone link is bandwidth-bound.
updatedAt: "2026-10-18T23:41:06.514Z"
---

## Log
- 2026-10-12T01:51:09.617Z: [claude] Won't do (2026-10-11 sort): no evidence the phone link is bandwidth-bound; needs a native build for speculative gain.
