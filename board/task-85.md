---
id: task-85
title: "Core: shell background processes belong to the session, not the turn"
column: todo
position: 1
priority: high
tags:
  - processes
  - shell
  - core
parentId: epic-8
createdAt: "2026-10-16T17:24:28.069Z"
---

## Description
Core: background processes that outlive the turn.

- `shell` gains `background: true`, for a process meant to keep running after the reply (a server, a watcher). It returns a receipt with the job_id. The turn never waits or parks on it, and `end_turn` happens as if it were not there.
- Ownership moves from the turn's `Jobs` to the teammate's session. A background process survives the turn and later turns. It is stopped, with its whole process tree, when:
  - the session stops or is replaced,
  - the teammate is stopped or removed,
  - its grant (capability lease) is revoked,
  - the desk quits.
  None is orphaned (same guards as today's shell jobs).
- Cap: 4 background processes per teammate. A fifth is refused in words naming the running ones.
- `inspect_job` and `cancel_job` work on them from any later turn; `wait_jobs` refuses them, since they never finish.
- Each process keeps a bounded output tail (stdout and stderr) and its full output file, as jobs do. A listening port is noted when it can be read from the output (e.g. "Serving HTTP on 127.0.0.1 port 8770"). This is best effort: never guess one.
- When a background process exits on its own, the next turn's context says so once (state, exit code, output tail). It does not wake the teammate.
- `jobs::INSTRUCTIONS`: tell the model to use `background: true` for anything meant to keep running (servers, watchers, `open` targets), and never a foreground job.
- A foreground job still running when the turn parks keeps today's behaviour: it is real work.

Tests:
- A turn that starts `python3 -m http.server` in the background, then replies, ends with `end_turn`.
- The process is alive after the turn, and `cancel_job` from a second turn kills it.
- Session stop kills it.
- The cap refuses a fifth.
