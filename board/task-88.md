---
id: task-88
title: "End-to-end: serve, open, stop — one proof over the wire, and the docs"
column: in-progress
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
updatedAt: "2026-10-16T20:37:29.171Z"
relatedFiles:
  - crates/hotline-core/tests/processes.rs
  - crates/hotline-core/tests/common
  - crates/hotline-core/tests/voice.rs
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/mod.rs
  - crates/hotline-core/src/session/dm.rs
  - docs/sessions.md
  - docs/voice-calls.md
  - CHANGELOG.md
contract:
  status: in_progress
  validation:
    commands:
      - cargo test -p hotline-core --test processes
      - "test \"$(cargo test -p hotline-core --test processes -- --list 2>/dev/null | grep -c ': test$')\" -ge 2"
      - grep -qi 'running process' docs/sessions.md
      - grep -qi 'listening' docs/voice-calls.md
      - grep -q '0.39' CHANGELOG.md
      - make verify
      - make check
  constraints:
    - The harness lives in crates/hotline-core/tests/processes.rs and drives the real core over the wire; the call proof may live there or extend tests/voice.rs.
    - Check the tree is gone by pid/process group, not only by the port closing.
    - Docs say only what is; keep the dm.rs paragraph and docs in step with the code task-85 shipped.
  metrics:
    readyAt: "2026-10-16T19:53:01.083Z"
    pickedUpAt: "2026-10-16T20:37:29.171Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-16T20:37:10.611Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T20:37:12.417Z"
assignee: claude
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

## Log
- 2026-10-16T20:19:34.530Z: [claude] From task-85: a chapter restart, a grant change and session replacement all kill a held process by design; the docs should say so. tests/processes.rs already proves end_turn/outlive/stop/revoke over the wire, so extend it rather than duplicating.
