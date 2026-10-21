---
id: task-20
title: "Core: answer a socket's commands concurrently"
parentId: epic-5
column: in-progress
position: 0
priority: high
tags:
  - improvement
  - bro-236
createdAt: "2026-10-11T21:05:30.464Z"
description: |-
  `seated_inner` awaits each command before reading the next frame (`wire/mod.rs` \~1194), and the window uses one socket, so any slow command (session.start, models.list, settings.update reattach, computer probes, search) stalls every command behind it. 0.33.4 moved the worst offenders off the path; this removes the class.

  Fix: spawn each command as its own task and send replies tagged by id (the UI already correlates by id). Keep ordering only where it matters: commands that append (persona.create, settings.update, prompts) relative to later subscribes and to each other per teammate.

  Related, also from the audit:

  * `session.start` waits for the agent process and container (`session/agent.rs` \~245, \~373): answer once `Starting`, finish bring-up in a task.
  * `settings.update` with mcpServers restarts every session serially inline (`session/mod.rs` \~2120-2133): reply after writing, reattach concurrently in a task.

  Migrated from Linear BRO-236: https://linear.app/1broseidon/issue/BRO-236/core-answer-a-sockets-commands-concurrently (2026-10-11).

  ## Status (2026-10-11 review)

  Per-teammate lanes landed in 0.35.2 (wire/mod.rs ~1187-1260). Left: commands not tied to a teammate (models.list, settings.update, search) still run inline (~1226).
updatedAt: "2026-10-21T10:23:11.888Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T10:22:50.112Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T10:22:51.912Z"
assignee: codex
---

## Log
- 2026-10-19T01:32:26.778Z: [claude] Follow-ups from task-55 (epic-14): (1) A command answered inline that hangs still stops the 90 s silence timer, because the timer isn't selected while an inline command is answered. (2) A stalled seat only has its file descriptor dropped; nothing resets the connection (no SO_LINGER reset).
- 2026-10-21T10:22:48.450Z: [claude] 2026-10-21 Pork Chop: lands on dispatch/epic-5 (now 3891214f, includes task-19's writer-held fold cache and task-46). Before done: merge dispatch/epic-5 into your branch, run make check in the foreground on the merged result, commit, and show a before/after measurement.
- 2026-10-21T10:27:01.166Z: [codex] Reader now dispatches independent commands before the ordered global worker; global writes/ping/subscriptions retain order and teammate lanes remain. Adding pending-command, silence and disconnect proofs plus before/after latency measurement. dispatch/epic-5 merge currently reports already up to date (0a1127a0).
