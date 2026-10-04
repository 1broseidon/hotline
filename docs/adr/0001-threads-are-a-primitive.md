# ADR 0001: Threads are a primitive

- Status: Accepted
- Date: 2026-10-03
- Design: [threads.md](../threads.md)

## Context

The room holds five kinds of conversation, and each was built on its own:

- **The DM** between the person and a teammate: the `Tape` stream, chapters,
  `sessionCheckpoints`, deliveries.
- **Peer exchanges** between teammates (ask and handoff): the `Thread(key)`
  pair stream, a sidecar, `exchange_pair` records on the room stream, a
  peer session per direction.
- **Side threads**: the `Side(id)` stream, a marker on the tape, a fresh agent
  of the teammate, live / parked / archived.
- **Voice calls**: an in-memory `Call` with an `Exchange` transcript, a
  `voice-dispatcher` pseudo-tape for desk calls, and handoffs into the DM by
  way of task-locals.
- **Subagent runs**: the `Run(id)` stream and a marker on the tape.

Each kind has its own write helper, agent builder, turn loop, resume
mechanism, idle sweep, restart settle, marker id scheme, permission answer
path, list summary, read receipts, live delta type and client hook, on the
desktop and again on the phone. The concrete inventory is in
[threads.md § Today](../threads.md#today).

The split shows up as bugs and gaps, not only as code size:

- A permission card raised in a peer turn or a subagent run has no answer
  path. One raised in a side thread never reaches the phone as a push, and
  never sets the roster's "waiting".
- A direct voice call leaves nothing behind. Its transcript lives in memory
  and is gone when the call ends or the desk restarts. It cannot be searched.
- Only the DM is indexed for `search_thread`. Peer threads and runs are not,
  and a side thread appears only as one synthetic line.
- Every client feature lands up to five times, on two clients. The side
  thread pane only began to look like the main chat once it reused the main
  transcript.
- Provenance is carried in id prefixes (`handoff:`, `exchange-result:`,
  `human-answer:`, `voice:<call>:<seq>:…`) and parsed back inside the turn
  loop.

## Decision

A **thread** becomes the one primitive for a conversation in the room: a
persisted, append-only stream with participants, an optional parent, a
lifecycle state and an agent binding. Each existing kind is a **policy** over
it, not a separate subsystem.

Every thread goes through one write path. That path appends, indexes for
search, raises and routes cards, pushes to the phone, mirrors to a live call
and tracks unread. It is served by one runtime: one agent builder, one turn
queue, and one resume mechanism, which is a stored session id with a
transcript fallback. It has one lifecycle, `live → parked → closed`, with one
sweep and one restart settle. It links to its parent with one typed link
event, and results flow back as deliveries with typed provenance. The wire
exposes `thread.*` commands, one `{thread: id}` subscription and one delta
type, and each client has one hook and one conversation view.

| Kind | Parent | Agent | Policy notes |
| --- | --- | --- | --- |
| DM | none (root, one per teammate) | the teammate's main session | chapters stay as segments of this thread |
| Side | the DM | a fresh copy of the teammate | no computer; parks; at most two live |
| Peer ask | both DMs | the target's peer agent, bound to the thread | first-contact approval; exchange cap |
| Peer handoff | both DMs | none; delivers into the target's DM | the result is a delivery back |
| Call | the DM | the voice front, delegating to the DM | voice modality; persisted |
| Run | the DM | a child agent, scoped lease | read-only to the person |

These stay as they are:

- **Authority.** A thread's lease derives from its parent's, never wider. A
  side or run cannot widen what its teammate can reach. Policies can only
  narrow, as `security.md` already requires.
- **Files on disk.** The existing stream files are read in place. The
  primitive is introduced over them, and no on-disk migration is part of this
  decision.
- **The wire.** Old commands stay as aliases until phones on older builds have
  aged out.

## Consequences

**Better**

- Approvals, pushes, "waiting", unread and search behave the same everywhere,
  because they happen in one write path.
- Calls are saved and searchable. A call can be reopened as a transcript.
- Resume works the same way for every agent-backed kind.
- A client feature is built once per client. Mobile parity costs one hook
  and one view, not five.
- A new kind of conversation is a new policy, not a new subsystem.

**Costs and risks**

- The refactor runs through `session/mod.rs`, which is about 4,800 lines, and
  the exchange worker. It has to land as small PRs behind the existing tests,
  with the DM ported last.
- Two names collide today: the `Thread(key)` pair stream is renamed in code to
  `Pair` and keeps its file layout.
- Until the aliases go, the wire carries both shapes.
- Chapters stay specific to the DM. Making them generic is out of scope. A
  side thread that grows long enough to need chapters should become a
  teammate.

## Alternatives considered

- **Keep the kinds separate, extract shared helpers.** This is cheaper now,
  but it leaves five lifecycles and five client hooks. It also doesn't fix
  the gaps that come from each kind choosing its own subset of behaviour,
  such as cards without an answer path or calls that aren't saved.
- **One stream file for everything with a kind field.** This makes the data
  model uniform, but it forces a migration of every room's history for no
  behavioural gain. The primitive is about behaviour; storage can stay where
  it is.
- **Make side threads and calls chapters of the DM.** This would put
  parallel work into a sequential structure. A side thread runs alongside the
  main conversation by design.
