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
updatedAt: "2026-10-16T19:53:01.081Z"
relatedFiles:
  - crates/hotline-core/src/contract.rs
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/wire/tests.rs
  - crates/hotline-core/src/session/mod.rs
  - ui/src/generated/contract.ts
  - ui/src/activity.ts
  - ui/src/components/Dock.tsx
  - ui/src/components/Work.tsx
  - ui/src/index.css
contract:
  status: ready
  validation:
    commands:
      - "test \"$(cargo test -p hotline-core process_chip -- --list 2>/dev/null | grep -c ': test$')\" -ge 3"
      - cargo test -p hotline-core process_chip
      - grep -q 'processes' ui/src/generated/contract.ts
      - grep -q 'process.stop' ui/src/generated/contract.ts
      - make check
  constraints:
    - "Name the new wire/contract tests process_chip_<what> and cover: SessionInfo.processes sent on change, process.stop refused for a non-owner and killing for the owner, process.output returning the tail."
    - The chip reuses the subagents chip's place and style; find it in ui/src/components (Dock.tsx/Work.tsx) rather than adding a new component tree.
    - "Do not change ui/src/activity.ts shownState semantics: a running process never makes the session read working."
  metrics:
    readyAt: "2026-10-16T19:53:01.083Z"
---

## Description
Contract and desk UI: running processes show as chips.

- `SessionInfo` gains `processes: [{id, title, startedAt, state}]`, where `title` is the command clipped to one line. It is sent on change like `awaitingSubagents`, and a client that predates it ignores it. No port: nothing is parsed out of output.
- `process.stop {personaId, id}` stops one, under the same owner checks as other session commands. Output is read with `process.output {personaId, id}`, which returns the tail.
- Desk: a chip in the same place and style as the subagents chip.
  - Label: the short command ("http.server 8770").
  - Tooltip: "Running process" (2-3 words, per ui rules).
  - Click opens a small panel with the output tail, the full command, and Stop.
  - Stop is immediate and the chip goes when the process is gone.
  - With several processes, one chip with a count opens the list.
- The session's state is unaffected: idle with a process running reads idle, with the chip beside it.
- No fake UI: the chip only appears for real processes from `SessionInfo`.

Run `make check`, including the contract.ts sync (watch for truncation).
