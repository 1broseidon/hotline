---
id: task-1
title: "Handoff page names the wrong teammate: \"The Dude handed this to The Dude\""
column: todo
position: 0
priority: high
tags:
  - bug
  - ui
  - threads
relatedFiles:
  - ui/src/components/Dock.tsx
createdAt: "2026-10-11T20:47:12.018Z"
---

## Description
Opening a handoff from the handing teammate's conversation (The Dude → Pork Chop) shows "The Dude handed this to The Dude. You can read along. Talk to The Dude about it ›", and the receiver's reply ("On it.") is labelled The Dude.

Cause: ui/src/components/Dock.tsx:510 (HandoffBar) and the thread page use the dock's `teammate` (whose conversation is open) as the receiver. Opened from the opener's side, dock teammate == opener. The receiver should come from the thread itself (its owner), not the dock; same for the speaker labels on the thread's lines.

Done when: from either side the bar reads "<opener> handed this to <receiver>", both faces are right, and lines are labelled by who said them.
