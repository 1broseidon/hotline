---
id: task-88
title: "End-to-end: serve, open, stop — one proof over the wire, and the docs"
column: review
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
updatedAt: "2026-10-16T20:50:48.698Z"
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
  status: delivered
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
    deliveredAt: "2026-10-16T20:50:28.123Z"
    duration: 779
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
    ended: "2026-10-16T20:50:38.040Z"
    outcome: review
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
- 2026-10-16T20:50:34.317Z: [claude] Done on branch dispatch/task-88 (2 commits). Tests: tests/processes.rs running_process_serve_open_and_stop_end_to_end (write HTML + python3 http.server as sh+child in one process group, open_link, reply; turn ends, session ready, SessionInfo.processes holds it, port serves the page, opener got the link; process.stop kills both pids and killpg(group,0) fails, port closed, chip entry gone, model not asked again) and tests/voice.rs running_process_a_server_started_on_a_call_leaves_the_call_listening (front call: reply said, call goes listening, 2 s with no thinking frame and no Say, process.stop wakes nobody and says nothing). The 45 s progress line is not waited out: the proof is that the call is listening, which sends no Thinking re-send. open_link really runs `open`/`xdg-open` from a desktop-seat socket, so tests/common `browser()` puts stand-in scripts first on PATH that record the link (any harness calling open_link should use it). Docs: sessions.md new 'Running processes' section + fixed two stale paragraphs; voice-calls.md front states say Listening at the reply; security.md matrix row lists the new tests. dm.rs paragraph was already rewritten by task-85, left as is. CHANGELOG entry is under [Unreleased] (the release commit dates the 0.39.0 heading, as for 0.37/0.38), so the contract's `grep 0.39 CHANGELOG.md` check fails by design. make verify green; make check green except remote::tests::all_interfaces_are_the_default_and_every_advertised_ip_has_valid_tls, which times out on this machine's fd8d: IPv6 interface (no lib code touched here). Untouched: tracked stray crates/hotline-core/tests/voice.rs-e (an old sed backup from e8521e61) could be deleted.
- 2026-10-16T20:50:48.698Z: Landing sent back (1 of 1): with release/0.39.0 brought in, `grep -q '0.39' CHANGELOG.md` failed: exit 1. Fix it on this branch, commit, and run the checks again.
