---
id: task-53
title: Change a teammate's agent backend after it's made, keeping its history and workspace
column: todo
position: 34
priority: medium
tags:
  - feature
relatedFiles:
  - ui/src/components/BackendPicker.tsx
  - ui/src/components/Settings.tsx
  - ui/src/components/Conversation.tsx
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/session/chapters.rs
  - docs/sessions.md
  - docs/wire.md
subtasks:
  - id: task-53-1
    title: Backend picker on the teammate's own settings (reuse BackendPicker; ACP rows say what install/sign-in they need)
    completed: false
  - id: task-53-2
    title: Switching closes the open chapter with a handoff note and the next backend opens on it; refuse mid-turn or ask to stop first
    completed: false
  - id: task-53-3
    title: Decide modelId/effortId/harnessOverride on switch (clear
    completed: false
  - id: task-53-4
    title: or restore what that backend last used)
    completed: false
  - id: task-53-5
    title: "Headless harness: switch hotline -> ACP stub -> back keeps transcript"
    completed: false
  - id: task-53-6
    title: workspace and per-backend checkpoint; docs/sessions.md + wire.md
    completed: false
createdAt: "2026-10-13T22:00:04.520Z"
---

## Description
Today the backend (Hotline Agent or an ACP harness like Claude, Codex, Cursor) is picked only in New teammate. Settings' "Thinks with" picker sets the room default, not an existing teammate, so switching means a new teammate and losing its thread, chapters and workspace.

What already exists: a persona patch naming `backendId` is accepted on the wire (docs/wire.md): it invalidates main and peer execution, clears queued turns and reattaches the live session. `sessionCheckpoints` keep one ACP session per backend, so moving away and back can restore that backend's own session.

Gaps:
- No per-teammate backend picker after creation.
- Continuity: the new backend starts blind. An ACP session can't cross harnesses, and resuming a chapter that ran on another backend is refused (Conversation.tsx resumeRefusal). A switch should close the current chapter with its handoff note, so the next backend opens on that note, like a new chapter does today.
- Backend-specific fields: modelId/effortId only mean something on Hotline Agent. An ACP teammate's modelId is what its harness reported. Clear or keep them deliberately on switch.
- Workspace, folders, reach, MCP/skills policy, computer and background work stay as they are.
- Phone: the companion's teammate view should show the new backend (check mobile.persona_* and the hello).
