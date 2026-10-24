---
id: task-287
title: Switch a teammate's agent (backend) from the model picker, waiting for idle, with the conversation carried over
column: todo
position: 8
priority: high
tags:
  - accounts
  - backends
  - picker
  - core
parentId: epic-25
createdAt: "2026-10-24T06:31:27.713Z"
---

## Description
George, 2026-10-24 01:31: agents should be able to switch backends, and it should be part of the new model picker, not hidden in the teammate's settings pane. Scenario S10 (added to docs/design/accounts-scenarios.html).

Today: Persona.backendId is set at creation; 'Thinks with' in the inspector. kelvinymoura found (issue #199) that persona.update {backendId} is accepted by the core but does not wait for idle, and A→B→A can resume a stale session and report it as Restored.

Picker shape: the top level lists Hotline Agent's accounts (as in the approved mock) and then each installed harness (Claude Code, Codex, Gemini, Cursor, Grok…) with its mark; a harness row opens its models (the harness's own list once it has reported one, else 'Default' for its own choice).

Done when:
- Picking a model under another agent switches the teammate's backend and model together; the switch waits until the teammate is idle and lands on its next turn. (session harness test)
- The new backend starts a fresh session seeded from the chapter's conversation, as a chapter boundary does; it never resumes a stale session of that backend and never reports 'Restored'. (harness test: A→B→A)
- A harness that is not installed shows in the picker as unavailable with what is missing, and cannot be picked. (UI test)
- The teammate pane's 'Thinks with' shows the same choice and stays in step. (UI test)
- The phone can do the same switch (hotline-mobile task-52 grows to cover it).
