---
id: task-88
title: "End-to-end: serve, open, stop — one proof over the wire, and the docs"
column: todo
position: 4
priority: high
tags:
  - processes
  - tests
  - docs
parentId: epic-8
createdAt: "2026-10-16T17:24:29.702Z"
dependsOn:
  - task-85
  - task-86
updatedAt: "2026-10-16T17:55:46.730Z"
---

## Description
One end-to-end proof and the docs.

- Integration test over the real wire (fixture model):
  1. The teammate writes an HTML file, starts `python3 -m http.server`, calls `open_link` on it, and replies without waiting.
  2. The turn ends and the session reads idle.
  3. `SessionInfo.processes` holds the server, and its port answers HTTP.
  4. `process.stop` kills it: the port closes and the chip's entry is gone, and the teammate is not woken.
- The same flow on a direct front call (scripted front): the call goes Listening when the reply is said, and no further Thinking snapshot (`voice/mod.rs` `thinking_again`) is sent while the server runs. The "Still on it" words are the front model's own filler while the call thinks; once the call is Listening there is nothing to silence.
- Docs: docs/sessions.md says what a running process is (a shell job still running when the turn parks), who owns it, when it stops, and that its exit wakes the teammate once. docs/voice-calls.md says a call goes Listening at the reply. The `dm.rs` paragraph about a command left going is rewritten. Add a CHANGELOG line for 0.39.
