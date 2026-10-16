---
id: task-85
title: "Core: a shell job still running when the turn parks becomes a session-owned process"
column: todo
position: 1
priority: high
tags:
  - processes
  - shell
  - core
parentId: epic-8
createdAt: "2026-10-16T17:24:28.069Z"
updatedAt: "2026-10-16T19:52:50.937Z"
relatedFiles:
  - crates/hotline-core/src/driver/rig/turn.rs
  - crates/hotline-core/src/session/jobs.rs
  - crates/hotline-core/src/session/dm.rs
  - crates/hotline-core/src/session/mod.rs
  - crates/hotline-core/src/session/lifecycle.rs
  - crates/hotline-core/src/session/stopping.rs
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/voice/mod.rs
  - docs/security.md
---

## Description
Core: shell jobs that outlive the turn.

The rule, in one place (`driver/rig/turn.rs`, where the loop parks today): when the model has said its reply and made no `wait_jobs` call, every shell job still running is handed to the teammate's session and the turn ends with `end_turn`. The turn parks only for subagents, as it does now. No `background` flag, no timer: the model's own choice not to wait is the signal. `wait_jobs` keeps today's behaviour.

- Ownership moves from the turn's `Jobs` to the teammate's session. A process survives the turn and later turns. It is stopped, with its whole process tree, when:
  - the session stops or is replaced (a grant change rebuilds the session, so it kills the process; that is by design),
  - the teammate is stopped or removed,
  - its grant (capability lease) is revoked,
  - the desk quits.
  None is orphaned (same guards as today's shell jobs).
- Cap: 4 processes per teammate, enforced at launch: once the teammate holds 4, `shell` refuses a new command in words naming the running ones (id and command), so the model hears it at once and can `cancel_job` one. The turn's own jobs keep their own limit. The model is told the cap in `jobs::INSTRUCTIONS`.
- `inspect_job` and `cancel_job` work on a process from any later turn; `wait_jobs` refuses one, since the turn that could wait on it is over.
- Each process keeps a bounded output tail (stdout and stderr) and its full output file, as jobs do.
- When a process exits on its own, its result (state, exit code, output tail) wakes the teammate once, as a subagent's report does, so a build the model did not wait on is still reported and a server that died is said. A stop from the chip or `cancel_job` does not wake it. When the teammate is on a live call, the turn it wakes is heard on the call as well as written, the way a reply to a hand-off is (check how the call relays a turn with no voice origin; today it would only reach the chat).
- `session/dm.rs`: the line "a command left going is work" goes; at park no shell job is in the turn.
- `jobs::INSTRUCTIONS`: say that a job still running when you reply keeps running on its own, its result arrives as a later turn, and `wait_jobs` is for a result you need before replying.

Security (docs/security.md applies; a process now outlives the turn): the PR states default, grant source, enforcement points, tests and residual risk. Residual risk: a model that neither waits nor means to leave a process gets its result one turn later, at one model request per exit.

Tests:
- A turn that starts `python3 -m http.server`, then replies, ends with `end_turn` at once.
- The process is alive after the turn; `cancel_job` from a second turn kills it.
- A turn that starts `sleep 2; echo done` and replies ends; two seconds later the teammate wakes once with `done`.
- Session stop kills it. Lease revocation kills it.
- With four processes held, a fifth `shell` launch is refused naming the four.
- On a live call (scripted front), a process exiting wakes a turn whose reply the front is given to say.
