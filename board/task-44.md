---
id: task-44
title: "Desktop: @ picker with mention tokens"
column: backlog
position: 27
priority: high
tags:
  - composer
  - ui
  - mentions
  - 0.38.0
relatedFiles:
  - ui/src
parentId: epic-4
createdAt: "2026-10-11T21:25:35.162Z"
updatedAt: "2026-10-14T20:05:26.179Z"
---

## Description
Typing @ at any position opens the room's teammates by public name with face/initial and state (idle, working, waiting, stopped); filters as you type, handles names with spaces, arrows to pick, Tab/Enter accept, Esc dismiss. An accepted mention is a token: one unit, backspace removes it whole, sent as a mention to the desk.
