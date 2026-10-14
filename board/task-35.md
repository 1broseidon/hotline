---
id: task-35
title: "Threads cleanup: last markers become link events, plus the epic's leftovers"
column: backlog
position: 19
tags:
  - bro-199
createdAt: "2026-10-11T21:05:58.997Z"
description: |-
  One sweep applying each kind's `idle` policy; one restart settle (park/close orphans, expire dead cards); one closing note via the chapter summariser. A thread with a parent writes one `link` event `{thread, kind, title, state, outcome}` rewritten by id — replacing `side:`, `subagent:`, `xthread:`, `exchange-paused:` markers. Results come back as deliveries with `from: {thread, kind, request}` instead of id-prefix parsing (`handoff:`, `exchange-result:`, `human-answer:`, `voice:…`) in the turn loop.

  **Done when:** the four settle functions and four sweeps are one each, and the turn loop has no id-prefix parsing for ported kinds. Old markers on existing tapes still render.

  docs/threads.md § One lifecycle, § Typed links, phase 4.

  ---

  ## Status at 0.36 (backlog review)

  Landed: link marker, one sweep and one settle (5c18b32, 5ba2fd9). The rest of the Threads epic (BRO-195) shipped in 0.32.0; this is now the home for what's left (docs/threads.md \~642-646):

  * Turn `xthread:` (`session/peers.rs` \~1377) and `exchange-paused:` (`session/exchanges.rs` \~301) markers into link events; drop the last id-prefix parse in `voice/mod.rs` \~189.
  * `search.thread` hits open their thread.
  * Move the inspector's Threads list from `peers.list` to `thread.list`.
  * Retire the `side.*`, `peers.*` and per-kind delta aliases once older phones age out.
  * Mark phase 10 Done in docs/threads.md.

  Migrated from Linear BRO-199: https://linear.app/1broseidon/issue/BRO-199/threads-cleanup-last-markers-become-link-events-plus-the-epics (2026-10-11).

  ## Status (2026-10-11 review)

  The rest of the Threads epic shipped in 0.32.0; this card holds the leftovers (already in the description).
updatedAt: "2026-10-14T20:05:22.293Z"
priority: medium
---
