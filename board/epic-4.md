---
id: epic-4
title: "Composer: @ teammates and / commands anywhere in a message (0.38)"
type: epic
column: backlog
position: 6
priority: high
tags:
  - feature
  - composer
  - 0.38.0
  - bro-136
createdAt: "2026-10-11T21:25:33.991Z"
updatedAt: "2026-10-14T20:05:15.943Z"
---

## Description
Target: 0.37.0. George's 2026-10-11 request supersedes the original start-only slash palette: @ teammate autocomplete and known / commands work anywhere in a message.

Mentions: picker uses public names, face and state, supports spaces and keyboard/touch selection; an accepted mention is one removable token and carries the resolved teammate id. Plain text from older clients remains compatible. Unknown slash text, paths, and/or and URLs remain text. Known slash entries include Hotline utilities, granted skills and ACP commands, with execution vs pass-through stated by one core catalogue. Decide the order of app commands relative to the accompanying prompt before implementing mixed messages.

Source audit: SessionSnapshot.slash_commands is initialized empty (session/mod.rs:351); no ACP available_commands_update handler or mention-token composer exists. None of the five desktop/core children is complete.

Sequence: task-41 ACP command updates → task-42 common catalogue → task-45 desktop slash picker; task-43 resolved mention delivery → task-44 desktop mention picker can proceed independently. Initial utility actions reuse chapter start/resume, schedule list/cancel and search. Keep new-chapter undo and touch/keyboard accessibility.

Done when both desktop and phone composers satisfy the behavior, ACP commands appear, resolved mention ids survive the wire, and release notes describe 0.37.0. The phone composer implementation must be tracked on the hotline-mobile board before this epic closes; the current children cover only the core and desktop.

Migration source: BRO-136.
