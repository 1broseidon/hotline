---
id: task-38
title: "Calling the desk: a switchboard that puts you through, with the teammates involved on screen"
column: todo
position: 40
tags:
  - feature
  - bro-273
createdAt: "2026-10-11T21:06:01.471Z"
description: |-
  From George, 2026-10-09, on a call after 0.36.0.

  **Today:** "Call the desk" still uses the old full-screen call, with the call assistant (router) answering for everyone and no memory. Calls to a teammate (0.36) talk to the teammate's own session, with two-version replies and the call card in its conversation, and that experience is much better.

  **Want: the desk as a switchboard**

  1. **Full screen stays**, since there's no single conversation to attach to, but it gets restyled in the 0.36 call-card language.
  2. **Route, then hand over.** You say what you need or who you want. The router picks the teammate, or asks which one when it's unclear. The call then becomes a normal teammate call (same call, no hang-up), running in that teammate's own session with two-version replies. The router doesn't answer questions itself. Open: whether a general question should go to a default teammate the person picks.
  3. **Mid-call hand-offs:** "put me through to Jeeves" switches teammates without hanging up.
  4. **Several teammates at once:** "tell Ghost and Giddy to take care of this" sends the task to both (as each one's turn, via the room's teammate messaging). The call stays with the switchboard, or with a lead the person names.
  5. **The teammates involved, on screen.** The call screen (desktop and phone) shows the avatars of the teammates on the call or working on what was asked. Each shows a live state (listening, working, done) and is tappable to open that teammate's conversation. They appear as they're pulled in ("Ghost and Giddy" → two faces) and stay while they work.
  6. **Same on desktop and phone.** Reuse the 0.36 pieces: the call card, the "Said on the call" line, keep-listening-while-working, and Bluetooth-safe call audio.

  **Notes:**

  * The router stays the "Call assistant" in Settings (it also rewrites untagged replies on teammate calls).
  * The wire needs a way to switch the call's target mid-call, and a list of involved teammates with their states for the call screen.
  * Budgets: routing is the call assistant's line; teammates' turns are their own.

  Migrated from Linear BRO-273: https://linear.app/1broseidon/issue/BRO-273/calling-the-desk-a-switchboard-that-puts-you-through-with-the (2026-10-11).
updatedAt: "2026-10-11T21:06:01.766Z"
---
