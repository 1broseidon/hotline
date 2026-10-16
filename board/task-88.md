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
updatedAt: "2026-10-16T18:54:12.227Z"
---

## Description
One end-to-end proof and the docs.

- Integration test over the real wire (fixture model):
  1. The teammate writes an HTML file, starts `python3 -m http.server`, calls `open_link` on it, and replies without waiting.
  2. The turn ends and the session reads idle.
  3. `SessionInfo.processes` holds the server, and its port answers HTTP.
  4. `process.stop` kills it: the port closes, no process of its tree is left (check by pid/process group, not only the port), the chip's entry is gone, and the teammate is not woken.
- The same flow on a direct front call (scripted front): the call goes Listening when the reply is said, and while the server runs the front is given no progress line (none of `voice/front.rs` `TEAMMATE_PROGRESS`, e.g. "Still on it."). Those lines are the bridge's, given each time `voice/mod.rs` `thinking_again` re-sends a Thinking snapshot (every 15 s); a call that is Listening sends none.
- Docs: docs/sessions.md says what a running process is (a shell job still running when the turn parks), who owns it, when it stops, and that its exit wakes the teammate once. docs/voice-calls.md says a call goes Listening at the reply. The `dm.rs` paragraph about a command left going is rewritten. Add a CHANGELOG line for 0.39.
