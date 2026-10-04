# Threads

A thread is a conversation the room keeps: an append-only stream with the
people and teammates in it, a parent it belongs to, a state, and the agent
that answers in it. The DM with a teammate, a side thread, an exchange
between two teammates, a voice call and a subagent run are all threads.
They differ only in policy. The decision and its trade-offs are in
[ADR 0001](adr/0001-threads-are-a-primitive.md); this document is the
design and the plan.

This describes the target. Until the phases below land, each kind still runs
on its own code. [Today](#today) is the map of that code.

## The model

```rust
pub struct Thread {
    pub id: ThreadId,
    pub kind: ThreadKind,            // Dm | Side | Pair | Call | Run
    pub parent: Option<ThreadLink>,  // the thread and the event it hangs off
    pub participants: Vec<Participant>, // Person | Persona(id) | Voice(persona)
    pub state: ThreadState,          // Live | Parked | Closed(End)
    pub title: Option<String>,
    pub binding: Option<AgentBinding>, // backend, session id, when it was saved
    pub note: Option<Note>,          // the closing note, chapter-shaped
}
```

A thread's events are the `TranscriptEvent`s the DM already uses. A kind adds
no new event shapes for its conversation, so one transcript renders every
thread.

**Policy** is what a kind decides. It is a value, not a subclass:

| Policy | What it decides |
| --- | --- |
| `seed` | The context a fresh agent gets: preamble, the parent's chapter note, the parent's tail, the thread's own history. |
| `tools` | Which teammate tools are served. Always a subset of the parent's. |
| `lease` | How authority derives from the parent: same, scoped, or without the computer. Never wider. |
| `idle` | When an idle thread parks or closes, and whether a restart parks or closes it. |
| `limit` | How many may be live per teammate, and what happens at the cap. |
| `surface` | What the parent shows: a link marker, a delivery on close, a push for cards. |
| `answer` | Who may answer cards raised in it: the person, or nobody (they expire). |

## The shared machinery

**One write path.** `Threads::write(thread, event)` is the only way an event
reaches a thread. It:

- appends to the stream
- indexes the event for search
- raises cards and routes them to the person's answer path, or expires them
  when the policy says nobody answers
- pushes to the phone when the policy surfaces cards
- mirrors to a live call on the parent DM
- updates unread and the roster's `waiting` flag

The tape path is the only one that does most of this today.

**One runtime.**

- **Agent builder.** `Threads::agent(thread)` builds the agent from the
  binding and the policy. It makes the folder, materializes `AGENTS.md`,
  handles the computer, and picks the preamble and history.
- **Turns.** `Turns` and the turn loop exist once and are keyed by thread.
- **Resume.** It is one mechanism. A binding's session id is resumed through
  the ACP child when it can reopen it. Otherwise the agent is seeded from the
  thread's own stream: history for Hotline Agent, a fenced transcript for an
  ACP child.

**One lifecycle.**

- **States.** `Live` has an agent. `Parked` has no agent, and saying
  something wakes it. `Closed` is read-only until continued.
- **Sweep and settle.** One sweep applies each thread's `idle` policy. One
  restart settle parks or closes orphaned threads and expires dead cards.
- **Closing note.** Closing writes the note through the same summariser
  chapters use.

**Typed links.**

- **Link marker.** A thread with a parent writes one `link` event on the
  parent: `{thread, kind, title, state, outcome}`. It is rewritten by id as
  the thread changes. This replaces the four marker id schemes (`side:`,
  `subagent:`, `xthread:`, `exchange-paused:`).
- **Deliveries.** A result flows back as a delivery whose provenance is a
  field (`from: {thread, kind, request}`). The turn loop no longer parses id
  prefixes (`handoff:`, `exchange-result:`, `human-answer:`, `voice:…`).

**One wire surface.**

- **Commands.** `thread.list | open | prompt | cancel | park | close |
  continue | answer`.
- **Subscription.** `{thread: id}`, with one `ThreadDelta` for streaming text
  and thoughts.
- **Old shapes.** The existing commands (`side.*`, `peers.*`, the
  side-specific deltas) stay as thin aliases until phones on older builds have
  aged out. `voice.*` stays as the call control surface; the call's
  conversation is a thread.

**One client view per client.** `useThread(id)` and one conversation view
render any thread. They are already the main transcript and composer that the
side-thread pane reuses. On the phone it is one hook and one sheet in place of
`use-peer-thread`, `use-side` and `use-run`.

## Kinds as policy

| | DM | Side | Pair (ask) | Pair (handoff) | Call | Run |
| --- | --- | --- | --- | --- | --- | --- |
| Parent | — | DM | both DMs | both DMs | DM | DM |
| Agent | main session | fresh teammate | target's peer agent | none: delivers into the target's DM | voice front, hands off to the DM | child agent |
| Lease | the teammate's | without computer | dual, scoped | the target's | none of its own | scoped child |
| Idle | chapters (setting) | park at 3 h | park at 10 min | n/a | end at 10 min | ends with the run |
| Restart | resume | park | park; cut-off told to the sender | requeue or fail | close, transcript kept | cancel |
| Limit | one | two live, park the idlest | one turn in flight per pair | queued | one call | job limits |
| Cards | person | person, pushed | person, pushed (today: none) | in the target's DM | signalled, never answered by voice | expire (today: no resolver) |
| Search | indexed | indexed | indexed (today: no) | in the DMs | indexed (today: lost) | indexed (today: no) |

Chapters stay specific to the DM. A side thread long enough to need chapters
should become a teammate.

## Storage

The stream files stay where they are, and no history is migrated:

| Kind | File |
| --- | --- |
| DM | `transcripts/<persona>/<epoch>.jsonl` |
| Side | `sides/<id>.jsonl` |
| Pair | `threads/<key>.jsonl` + sidecar; renamed `Pair` in code |
| Run | `runs/<id>.jsonl` |
| Call | `calls/<id>.jsonl` (new) |

The thread records themselves (state, binding, title, note) live where each
kind keeps them today: the marker line, the sidecar, the persona record. They
are read through one `ThreadStore` trait. A later phase may consolidate them;
nothing here requires it.

## Phases

Each phase is its own PR, or a few, and keeps every existing test green. The
DM is ported last, because it has the most to lose.

1. **The type and the store.** Add `Thread`, `ThreadKind`, `ThreadState`,
   `ThreadLink`, `AgentBinding` and `ThreadStore` over the existing records.
   Rename the pair stream to `Pair` in code.
   *Done when:* every existing stream can be listed and loaded as a `Thread`
   with no behaviour change.
2. **One write path.** `Threads::write` with indexing, cards, push, call
   mirror, unread and waiting, applied first to sides and runs.
   *Done when:* side cards push to the phone and set the roster's `waiting`,
   and run cards expire instead of hanging.
3. **One runtime.** Merge the agent builders, `Turns` and the turn loops, and
   put resume behind one mechanism. Port sides first, then runs.
   *Done when:* there is one `Turns` and the side and run builders are gone.
4. **One lifecycle.** One sweep, one restart settle, one closing note, and
   the link marker with typed delivery provenance.
   *Done when:* the four settle functions and four sweeps are one each, and
   the turn loop has no id-prefix parsing for the ported kinds.
5. **Calls are threads.** Persist the call transcript to `calls/<id>.jsonl`,
   index it, link it to the DM, and let a closed call be read back.
   *Done when:* a call's lines survive a restart and `search_thread` finds
   them.
6. **Pairs are threads.** Port asks and handoffs. Give peer cards an answer
   path, index pair threads, and fold read receipts into the shared unread.
   *Done when:* a card raised in a peer turn can be answered, and pair
   threads are searchable.
7. **The DM is a thread.** Move the main session onto the runtime and the
   lifecycle, keeping chapters as DM policy.
   *Done when:* `run_turns` is the shared turn loop.
8. **The wire.** Add `thread.*`, `{thread: id}` and `ThreadDelta`, and alias
   the old commands. Regenerate the contract, and check it is complete.
   *Done when:* both shapes pass the wire tests.
9. **The desktop.** One `useThread`, and one view for the dock, peer threads,
   runs and calls.
   *Done when:* `useSide`, `useThread` (pair), `useRun` and their
   components are one hook and one view.
10. **The phone.** The same, in the mobile repo, behind a `threads2`
    capability so older desks keep working.
    *Done when:* the phone opens any kind of thread in one sheet.

## Today

The map this design replaces, as of `qa/sides-voice` (main, plus side
threads, resumable side threads, and the call voice with memory).

| Concern | DM | Side | Pair | Call | Run |
| --- | --- | --- | --- | --- | --- |
| Stream | `Tape` | `Side` | `Thread(key)` + sidecar + `exchange_pair` on Room | memory (`Exchange`); desk calls on a `voice-dispatcher` tape | `Run` |
| Write helper | `write_value` | `write_side` | `write_thread`, `exchange_thread_line` | `voice_record` | `append_run` |
| Agent builder | `start_now` | `bring_up` | `peer_session` | dispatcher front | `run_to_end` |
| Turn loop | `run_turns` + `Turns` | `run_side_turns` + `Turns` | `drive` | voice `run` | `drive_with` |
| Resume | persona checkpoints | marker `sessionId` | reseed from thread | none | none |
| Idle | `sweep_chapters` | `sweep_sides` (3 h) | `sweep_peers` (10 min) | own loop (10 min) | — |
| Restart | `settle_tapes` | `settle_orphaned_sides` | `reconcile_exchanges` + `recover_exchanges` | lost | `settle_orphaned_subagents` |
| Marker | — | `side:` | `xthread:`, `exchange-paused:` | `voice:` id grammar | `subagent:` |
| Cards answered by | `session.answer_permission` | `side.answer_permission` | nobody | never | nobody |
| Push / waiting | yes | no | no | fallback push | no |
| Search | indexed | archived marker only | no | desk calls only | no |
| Desktop hook | `useTape` | `useSide` | `useThread` | `voice/call.ts` | `useRun` |
| Phone hook | team state | `use-side` (unreleased) | `use-peer-thread` | `voice/call.ts` | `use-run` |

## Open questions

- **Groups.** Should a thread allow more than two personas, for example a
  team-wide thread? The model allows it, but no policy uses it yet.
- **Calls about a side thread.** Should a call be able to bind to a side
  thread rather than the DM? It falls out of `parent` for free once calls are
  threads.
- **Record consolidation.** Should the thread records move into one
  `threads.jsonl` index once every kind reads through `ThreadStore`? That
  decision belongs after phase 7.
