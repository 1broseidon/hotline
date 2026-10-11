---
id: task-22
title: "Window performance: streaming re-renders, progressive render and virtualization, cached room-wide lists"
column: todo
position: 24
priority: high
tags:
  - improvement
  - bro-241
createdAt: "2026-10-11T21:05:32.121Z"
description: |-
  Every animation frame of deltas notifies Conversation, Transcript, Dock and Work (`ui/src/tape.ts` \~215-255), even though the window shows only whole bubbles (`reveal.ts` wholeBubbles) and a step count. Each flush re-splits the full streaming text twice (quadratic over a long reply), builds events with `ts: Date.now()` (`Transcript.tsx` \~586, \~591), and reruns unmemoized `groupScheduled`/sides/`handoffsUnderway`/`activityOf`/`useCadence` passes.

  Also per frame: the Work card refolds all events with `stepRuns` and forces layout in a dependency-less layout effect under a `backdrop-filter` surface (`Work.tsx` \~44, \~132-135); the band reruns `resumeRefusal`, the jobs reduce and the menu (`Conversation.tsx` \~219-297).

  Fix: in `ThreadStore.apply` for words, compute the visible projection and notify only when it changes; stable `ts`; memoize blocks/sides/underway; memoize stepRuns and gate the follow effect; split Conversation so the band doesn't re-render with the body; solid background for `.work-float` while working.

  ---

  ## Merged in from BRO-242: progressive render and virtualization for long conversations

  Switching teammates remounts the conversation and renders all \~200 rows in one commit (`App.tsx` Conversation keyed by persona, `Transcript.tsx` \~310).

  * Render the last \~30 rows first and the rest in `startTransition`/idle; wrap `select()` so the rail highlight updates immediately.
  * Virtualize long tapes.
  * Scroll anchoring: `content-visibility: auto` with a 96 px placeholder jitters on WebKit, which lacks overflow-anchor (`index.css` \~2665). Add manual anchoring or measured virtualization.
  * Mermaid: draw only near the viewport (IntersectionObserver); skip the 300 ms settle for durable events.
  * Computer frames: blob URLs cached by event id instead of base64 in transcript state (`Transcript.tsx` \~1858).

  ## Merged in from BRO-244: cache room-wide lists the band and inspector refetch on every switch

  * `SessionPickers` refetches `models.efforts` and `backends.list` on every switch, so the effort picker shifts the band (`Pickers.tsx` \~48-85).
  * The inspector refetches `backends.list`, `voice.status`, `capabilities.options`, `computer.capacity`, `computer.status` per persona (`Teammate.tsx`).

  Fix: module-level caches for the room-wide ones, invalidated by `useModelsRevision`; refetch only per-persona computer status. Also replace `:has()` selectors on bubbles (`index.css` \~2671, \~2281) with a data attribute and a parent class.

  Migrated from Linear BRO-241: https://linear.app/1broseidon/issue/BRO-241/window-performance-streaming-re-renders-progressive-render-and (2026-10-11).
updatedAt: "2026-10-11T21:20:21.910Z"
---
