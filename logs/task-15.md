---
id: task-15
title: "Desktop: one unified list across every connected desk, with a desk filter (like the phone)"
priority: high
tags:
  - feature
  - bro-178
parentId: epic-3
createdAt: "2026-10-11T21:05:25.650Z"
description: |-
  Slice 1 of BRO-177. It's the foundation, and it's pure client work that can ship on its own.

  ## Today

  The window holds several desks (the local one plus paired servers, BRO-145) but shows **one at a time**. `DeskSwitcher` in the rail (`ui/src/components/Rail.tsx:428`) picks the active desk, and `desks.ts` keeps `activeDeskId()` with a `wireFor(deskId)` connection per desk. The phone already shows every desk's teammates in one list with a filter (BRO-112; mobile `DeskMenu` with per-desk status in the menu, mobile #39).

  ## Change

  * **One rail across all desks.** Every connected desk's teammates are in one list, ordered like the phone's (recency and waiting-on-you first), each row showing its desk only when more than one desk is connected (a quiet suffix or small tag, matching the phone's row).
  * **A desk filter instead of a switcher.** It says "All desks" by default, with per-desk items that include status (connecting spinner, offline warning, Reconnect), the same design as the phone's desk control (mobile #39). Choosing a desk filters the list, and it never swaps the whole window.
  * **Everything else follows the teammate, not a global "active desk".** Opening a teammate uses its desk's wire. Settings, the computer viewer, files, schedules and new-teammate all act on the selected teammate's desk. Settings' Desks section is where each desk is managed (no new screen, per BRO-175's constraint).
  * **Add teammate** asks which desk when more than one is connected, defaulting to the filtered desk or else the local one.
  * **Unread and "needs you"** count across all desks. The titlebar and badge are the sum; the background-desk unread from 1broseidon/hotline#107 becomes the normal case.
  * **Offline desks** keep their teammates listed, dimmed, with the desk's status in the filter (same as the phone), never vanishing (BRO-147).
  * **The narrow, faces-only rail** keeps working, with the desk shown on the hover card.

  ## Not in this slice

  Nothing server-side. Desks don't know about each other yet; that's linking (BRO-177 slices 2–4). This is only the window showing what it's already connected to.

  ## Done when

  With the local desk plus grizzly connected:

  * The rail shows both desks' teammates in one list.
  * Filtering to grizzly shows only grizzly's, and "All desks" shows everyone.
  * Chatting with a grizzly teammate and then a local one needs no switching.
  * Unread from either desk shows in the one list.
  * Taking grizzly offline dims its rows and shows the warning in the filter.

  Migrated from Linear BRO-178: https://linear.app/1broseidon/issue/BRO-178/desktop-one-unified-list-across-every-connected-desk-with-a-desk (2026-10-11).
updatedAt: "2026-10-13T07:31:40.805Z"
completedAt: "2026-10-13T07:31:40.805Z"
---

## Log
- 2026-10-13T07:31:31.785Z: [claude] On dispatch/epic-3: one rail lists every desk's teammates, with the desk named when more than one is shown and unreachable desks dimmed from cache. A desk filter replaces the switcher and shows link state. Opening a teammate opens it on its own desk, and pin, delete and edit act on that desk. ⌃1–9 follow the merged order, the badge counts unread across desks, and New teammate and Settings each have a desk picker. Tests are in ui/tests/team.test.tsx.
