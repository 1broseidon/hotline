---
id: task-72
title: Shorten every tooltip to 2-3 words (5 max)
column: in-progress
position: 0
priority: medium
tags:
  - ui
  - copy
  - 0.38.0
decisions:
  - decision-2
subtasks:
  - id: task-72-1
    title: Conversation.tsx:400 'Setting up the computer · N%'
    completed: true
  - id: task-72-2
    title: DesksSection.tsx:85 'Let this computer's teammates and this server's see and message each other'
    completed: true
  - id: task-72-3
    title: Dock.tsx:427 'Bring this thread back with what it remembers'
    completed: true
  - id: task-72-4
    title: Rail.tsx:674 'All desks · one can't be reached'
    completed: false
  - id: task-72-5
    title: Secrets.tsx:511 'Not stored any more. Untick it or store it again under Settings → Secrets.'
    completed: false
  - id: task-72-6
    title: SentFile.tsx:53 'Show in its folder on the server'
    completed: false
  - id: task-72-7
    title: ServerFiles.tsx:162 'Save a copy on this computer'
    completed: false
  - id: task-72-8
    title: Teammate.tsx:394 'N teammates are pinned. Unpin one first.' and 'Pinned teammates sit at the top of the team on every device'
    completed: false
  - id: task-72-9
    title: Welcome.tsx:244 'An AI coding tool you already have'
    completed: false
createdAt: "2026-10-15T23:17:58.177Z"
updatedAt: "2026-10-15T23:19:31.279Z"
---

## Description
Applies the ui.tooltip-length rule. Found by scanning every title= in ui/src on 2026-10-15; the phone has no hover tooltips. One subtask per offender, file:line as of release/0.38.0 528f87e5. Not counted: shortcut hints in parentheses, model names, and the faces-only rail's hover card, which previews a teammate's last line rather than labelling a control.
