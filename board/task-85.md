---
id: task-85
title: "Core: a shell job still running when the turn parks becomes a session-owned process"
column: in-progress
position: 1
priority: high
tags:
  - processes
  - shell
  - core
parentId: epic-8
createdAt: "2026-10-16T17:24:28.069Z"
updatedAt: "2026-10-16T19:53:21.467Z"
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
contract:
  status: in_progress
  validation:
    commands:
      - "test \"$(cargo test -p hotline-core running_process -- --list 2>/dev/null | grep -c ': test$')\" -ge 6"
      - cargo test -p hotline-core running_process
      - "! grep -n 'a command left going' crates/hotline-core/src/session/dm.rs"
      - make check
  constraints:
    - Name every new test running_process_<what it proves> so the contract can find them; the six in the description are the minimum.
    - Run `cargo test -p hotline-core` unfiltered (make check does) last, so ui/src/generated/contract.ts is whole.
    - "No background flag and no timer: the park in turn.rs is the only rule."
    - State default, grant source, enforcement points, tests and residual risk in the final note (docs/security.md).
  metrics:
    readyAt: "2026-10-16T19:53:01.083Z"
    pickedUpAt: "2026-10-16T19:53:21.467Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-16T19:53:08.888Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T19:53:10.142Z"
assignee: claude
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

## Log
- 2026-10-16T20:17:20.235Z: [claude] Done on branch dispatch/task-85 (make check green). At park in driver/rig/turn.rs, Jobs::release() hands every running shell job to session/jobs.rs `Processes` (owned by the InProcess driver) and the turn ends with end_turn; the card closes 'Still running on its own'. Only a driver whose wakes are subscribed takes processes: Driver::subscribe_wakes, subscribed for the DM only (Watches.wakes), so subagent runs, sides and peers still park on their own commands. A process that exits on its own sends a wake line; Room::watch_wakes queues it as a Hotline nudge (never on the tape) with voice = Calls::on_call(persona), so on a direct call the reply goes to the front. cancel_job and the chip-to-be (Processes::cancel) do not wake. wait_jobs refuses a process; inspect_job/cancel_job reach it from later turns, and the last 4 ended stay readable. Cap: launch counts the turn's running commands plus held processes, 4 in all, and the refusal names each id and command.
- 2026-10-16T20:17:23.578Z: [claude] Security (docs/security.md 'Running processes' + matrix row). Default: nothing newly granted; the command was launched under the shell grant and the turn's lease; only the DM session takes processes. Grant source: reach/shell, unchanged. Enforcement points: InProcess::invalidate -> Processes::stop_all (session stop, removal, grant change, which rebuilds the session by design, and revocation all call it); driver drop / desk quit drop the task and the shell's process-group guard kills the tree; launch cap in Jobs::launch; wakes are queued only while the session and its lease are current. Tests: tests/processes.rs running_process_* (4), tests/voice.rs running_process_that_ends_during_a_call_is_said_on_the_call, session/jobs.rs running_process_* (3). Residual risk: a model that neither waits nor means to leave a command gets its result a turn later, at one model request per exit; output is held in memory until exit, as for any job; a chapter restart replaces the session, so it kills a process too.
- 2026-10-16T20:17:26.435Z: [claude] For task-86/88: the room reaches processes through the driver; add a Driver method (e.g. processes() -> Option<Arc<Processes>>) for SessionInfo.processes, process.stop (Processes::cancel; it does not wake) and process.output. Processes::running() gives id/command/state; there is no startedAt yet (add it to the held entry). No live tail exists: RunCommand::run captures output only at exit (wait_with_output). process.output needs the shell to stream stdout/stderr into a bounded tail and a file while it runs. A change notification is needed for the chip: Processes has no watch channel yet (hold/exited are the two places to send one).
