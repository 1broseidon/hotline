---
id: epic-9
title: "ACP: Claude and Codex background tasks as running processes (AIR asyncTasks, experimental)"
type: epic
column: backlog
position: 44
priority: medium
tags:
  - acp
  - processes
  - experimental
createdAt: "2026-10-17T05:12:06.531Z"
---

## Description
Bring the 0.39 running-process chip to Claude and Codex teammates, experimental.

Research (2026-10-16): standard ACP has no way for a client to know about or stop an agent's background work. v1 client terminals (terminal/create…) are slated for removal in v2 (draft RFD), and the v2 Terminal Output RFD is display-only, with no running state and no stop. Both adapters we pin implement JetBrains' AIR extension instead:
- claude-agent-acp 0.78.0 (pinned) through 0.93.0;
- codex-acp 1.12.0 (pinned) through 2.2.2.

Opt in with `initialize` clientCapabilities `_meta: { jetbrains: { air: { version: 1, capabilities: ["asyncTasks"] } } }`. The agent then sends:
- `async_task_spawned` {asyncTaskId, name, taskType ("shell" for a command), description, canStop, toolCallId, outputFilePath};
- `async_task_progress`;
- `async_task_state_update` {state: running|paused|completed|failed|stopped}.

Stop with `_session/async_task/stop {sessionId, asyncTaskId}`. A backgrounded Bash card is marked backgrounded, and its terminal isn't shown as exited.

Scope:
- advertise asyncTasks behind an experimental setting (driver/acp.rs initialize_request);
- map shell tasks into SessionInfo.processes, so the desk chip and the phone's … rows work unchanged;
- process.stop calls the adapter's stop;
- process.output tails outputFilePath;
- label non-shell task types, or leave them out.

Folds in task-51 (a Claude Code child that finishes with a background job still running is marked done, and its later work stays invisible).

Open questions:
- Does Claude's adapter hold session/prompt open while a shell task runs, as it does for subagents?
- outputFilePath can arrive after the spawn event.
- Muse and Grok adapters don't implement it.

Sources: agentclientprotocol.com/protocol/v1/terminals, /rfds/v2/client-filesystem-terminal-capabilities, /rfds/v2/terminal-output; the adapters' dist (async-tasks.js, air-extension.js, acp-agent.js `_session/async_task/stop`).
