---
id: task-86
title: "Contract + desk: running processes show as a chip with output and Stop"
column: todo
position: 2
priority: high
tags:
  - processes
  - ui
  - contract
parentId: epic-8
createdAt: "2026-10-16T17:24:28.605Z"
dependsOn:
  - task-85
updatedAt: "2026-10-16T17:24:30.245Z"
---

## Description
Contract and desk UI: running processes show as chips.

- `SessionInfo` gains `processes: [{id, title, startedAt, port?, state}]`, where `title` is the command clipped to one line. It is sent on change like `awaitingSubagents`, and a client that predates it ignores it.
- `process.stop {personaId, id}` stops one, under the same owner checks as other session commands. Output is read with `process.output {personaId, id}`, which returns the tail.
- Desk: a chip in the same place and style as the subagents chip.
  - Label: the port when known ("Server :8770"), otherwise the short command ("http.server").
  - Tooltip: "Running process" (2-3 words, per ui rules).
  - Click opens a small panel with the output tail, the full command, and Stop.
  - Stop is immediate and the chip goes when the process is gone.
  - With several processes, one chip with a count opens the list.
- The session's state is unaffected: idle with a process running reads idle, with the chip beside it.
- No fake UI: the chip only appears for real processes from `SessionInfo`.

Run `make check`, including the contract.ts sync (watch for truncation).
