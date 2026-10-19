---
id: task-20
title: "Core: answer a socket's commands concurrently"
parentId: epic-5
column: backlog
position: 12
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
updatedAt: "2026-10-14T20:05:18.890Z"
---

## Log
- 2026-10-19T01:32:26.778Z: [claude] Follow-ups from task-55 (epic-14): (1) A command answered inline that hangs still stops the 90 s silence timer, because the timer isn't selected while an inline command is answered. (2) A stalled seat only has its file descriptor dropped; nothing resets the connection (no SO_LINGER reset).
