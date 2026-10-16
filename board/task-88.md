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
---

## Description
One end-to-end proof and the docs.

- Integration test over the real wire (fixture model):
  1. The teammate writes an HTML file, starts `python3 -m http.server` with `background: true`, calls `open_link` on it, and replies.
  2. The turn ends and the session reads idle.
  3. `SessionInfo.processes` holds the server with its port, and the port answers HTTP.
  4. `process.stop` kills it: the port closes and the chip's entry is gone.
- The same flow on a direct front call (scripted front): after the spoken reply the call is Listening and no "Still on it" line is given.
- Docs: docs/voice-calls.md and the session or jobs docs say what a running process is, who owns it and when it stops. Add a CHANGELOG line for 0.39.
