# Threads

A thread is a conversation the room keeps: an append-only stream with the
people and teammates in it, a parent it belongs to, a state, and the agent
that answers in it. The DM with a teammate, a side thread, an exchange
between two teammates, a voice call and a subagent run are all threads.
They differ only in policy. The decision and its trade-offs are in
[ADR 0001](adr/0001-threads-are-a-primitive.md); this document is the
design and the plan.

This describes the target. Until the phases below land, each kind still runs
on its own code. [Today](#today) is the map of that code. Phases 1 to 7 have
landed: [what is built](#built-so-far) says where, and where it differs from
what is written here.

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

| | DM | Side (work thread) | Pair (ask) | Call | Run |
| --- | --- | --- | --- | --- | --- |
| Parent | — | DM, and the opener's DM when a teammate opened it | both DMs | DM | DM |
| Agent | main session | the teammate's own agent, full tools | target's peer agent | voice front, hands off to the DM | child agent |
| Lease | the teammate's | the teammate's; the computer through an exclusive lease | dual, scoped; the same computer lease | none of its own | scoped child |
| Idle | chapters (setting) | park at 3 h | park at 10 min (`Room::sweep` row) | end at 10 min | ends with the run |
| Restart | resume | park | park; cut-off told to the sender | close, transcript kept | cancel |
| Limit | one | three live per teammate, park the idlest; a handoff queues when all are mid-turn | one turn in flight per pair | one call | job limits |
| Cards | person | person, pushed; `request_human` lands here | person, pushed, `peers.answer_permission` | signalled, never answered by voice | expire |
| Search | indexed | indexed | indexed under both teammates | indexed | indexed |

A handoff is a side thread: opened by a teammate instead of the person, with the
sender as its opener. Its result returns to the opener as a delivery.

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
   with no behaviour change. **Done.**
2. **One write path.** `Threads::write` with indexing, cards, push, call
   mirror, unread and waiting, applied first to sides and runs.
   *Done when:* side cards push to the phone and set the roster's `waiting`,
   and run cards expire instead of hanging. **Done.**
3. **One runtime.** Merge the agent builders, `Turns` and the turn loops, and
   put resume behind one mechanism. Port sides first, then runs.
   *Done when:* there is one `Turns` and the side and run builders are gone.
   **Done**, for sides and runs; the DM and pairs follow in phases 6 and 7.
4. **One lifecycle.** One sweep, one restart settle, one closing note, and
   the link marker with typed delivery provenance.
   *Done when:* the four settle functions and four sweeps are one each, and
   the turn loop has no id-prefix parsing for the ported kinds. **Done**, for
   sides and runs, with the others hooked in.
5. **Calls are threads.** Persist the call transcript to `calls/<id>.jsonl`,
   index it, link it to the DM, and let a closed call be read back.
   *Done when:* a call's lines survive a restart and `search_thread` finds
   them. **Done**, for direct calls.
6. **Work threads, and pairs on the shared path.** Handoffs and side threads
   become one kind: a work thread, opened by the person or a teammate, served by
   the target's own agent with the full toolset, sharing its computer through a
   lease, and returning its note to the opener. Asks stay lightweight pair
   threads, written, indexed and answered through the shared path.
   *Done when:* a handoff runs in its own thread while the target's DM answers
   the person, its result returns to the sender's DM, the computer lease
   serializes two threads, a card raised in a peer turn can be answered, and
   pair threads are searchable. **Done.** Peer read receipts stay a pair-stream
   concern (see below).
7. **The DM is a thread.** Move the main session onto the runtime and the
   lifecycle, keeping chapters as DM policy.
   *Done when:* `run_turns` is the shared turn loop. **Done**: `Room::run_queue`
   serves the DM and work threads, and `start_now` builds through
   `Room::thread_agent`.
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

## Built so far

**Phase 1.** `crates/hotline-core/src/thread/` holds `Thread`, `ThreadId`,
`ThreadKind`, `ThreadState`, `End`, `ThreadLink`, `AgentBinding`,
`Participant`, `Policy` and `ThreadStore`. `ThreadStore::load`, `list` and
`all` fold the records each kind keeps and write nothing. The pair stream is
`StreamId::Pair` in code, with the same files.

**Phase 2.** `Threads::write` is in `session/threads.rs`, beside the room
whose fields it touches. Sides and runs write through it (`write_side` and
`append_run` are gone, and so are the direct appends of their markers). The
search index tags a line from a thread (`message_threads`), keeps it across a
rebuild, and offers it to `search_thread`.

Where this differs from the design above:

- **`ThreadStore` is a struct, not a trait.** It has one implementation, and a
  trait can be cut from it when a second reads a different record.
- **`AgentBinding` has no save time.** No record keeps one, and nothing reads
  it. **`note` is the marker's text**, not a chapter-shaped `Note`, until the
  closing note is shared in phase 4.
- **`ThreadId` carries the kind**, so a `Thread` has no `kind` field of its
  own; `thread.kind()` reads it.
- **A pair has no parent**, because the model has one and a pair hangs off two
  DMs. A run's parent is found from the marker on the teammate's tape, since
  the run's own marker does not name its owner.
- **A record's word is the state.** A side or run a dead process left `Live`
  loads as `Live`, a pair is `Live` while its `exchange_pair` has a request
  running, and a DM is always `Live`. The room's settle still parks the
  orphans. The store cannot tell a stopped DM from a running one.
- **`write` takes the teammate too**: `write(thread, persona_id, event)`. A
  run's id does not say whose it is, and the index, the push and the roster row
  are all the teammate's.
- **`Policy` holds only what the write path reads**: `answer` and `surface`
  (push, call mirror, index). The other policies arrive with the phases that
  read them.
- **The DM, pairs and calls are not on the write path yet.** `write` hands a DM
  to the tape's own door and refuses the other two with a log line. Each is
  ported in its own phase.
- **Only a permission is routed as a card.** The other cards (`request_human`,
  a passkey) are written to the tape by the session that parks on them, not by
  a thread's agent.
- **Unread is not written.** A client counts it against each thread's newest
  line; a write's part is to land that line and wake the roster row. What the
  server could own is the phase 6 read receipts.
- **The window's search stays on the tape.** `search.thread` and `search.all`
  do not offer a thread's lines, because a hit there could not open: phase 9
  gives it somewhere to go. The agent's `search_thread` offers them, with a
  `thread` field on the hit.
- **The call mirror is the existing `voice.card`.** A card in a side thread
  reaches a live call the way one on the tape does; the mirror is not tested
  at the call level yet.

**Phase 3.** `Room::thread_agent` in `session/agent.rs` builds the agent of a
side thread or a run from its kind's `Policy`, which now also holds `seed`,
`tools`, `lease` and `computer`. `bring_up` and `run_to_end` call it, and
`side_agent` and the run's own builder are gone. `session/turns.rs` holds the
one `Turns` (generic over the line it queues, so the DM's `Wired` lines use it
too) and `Threads::turn`, the turn both a side's loop and a run take: it drives
the agent, writes through `Threads::write`, delivers a refusal, and emits a
side's live deltas. Resume is in the builder: the binding is read from the
thread's own record, a child is asked to reopen it, and otherwise the agent is
seeded from the thread's stream.

Where this differs from the design above:

- **The builder is `Room::thread_agent`, not `Threads::agent`.** The tools
  hold a `Weak<Room>` made from an `Arc<Room>`, and `Threads` borrows a `&Room`.
  It moves onto `Threads` if that borrow becomes an `Arc`.
- **The lease is made before the agent.** `lease_of(kind, parent)` derives it
  from the kind's policy, and the caller passes it in, because a run's drop
  guard has to revoke it even when the start is cancelled halfway. A side's
  `Independent` lease is its own epoch; a run's is `Scoped`; the DM's and a
  pair's rows are set but nothing reads them yet.
- **A thread's own history is read by the builder.** A policy `seed` decides
  whether a kind has any: a side does, a run does not, though its stream already
  holds its task and marker when it starts.
- **The run's one turn is `Threads::turn`, not a loop.** A run has no queue, so
  there is one `Turns` and no second turn loop for it to use; the loop over a
  queue stays in `run_side_turns` until the DM's `run_turns` can share it.
- **Not moved:** `start_now` (phase 7) and `peer_session` (phase 6) still build
  their own agents. The builder already grants the computer when a kind's policy
  says so, which is the part of `start_now` and `peer_session` they share, but
  that branch has no caller and no test until one of them moves. The skills
  index is not materialised by the builder: a side and a run share the main
  session's folder, where `start_now` already wrote it.
- **Resume is not tested beyond a thread's own record.** The existing
  side-thread tests cover a reopened session and a refused one; the selection
  rule is a pure function with its own test.

**Phase 4.** `session/lifecycle.rs` holds the lifecycle once. `Room::sweep`
applies each kind's `idle` policy (`Policy::idle`): a side thread is parked
there, and a run has none. It calls `sweep_chapters` and `sweep_peers` for the
DM and the pairs. `Room::settle` replaces `settle_tapes`,
`settle_orphaned_sides` and `settle_orphaned_subagents`: it expires dead cards on
every tape and pair, moves each live thread by the kind's `restart` policy, and
calls `reconcile_exchanges`. `Room::queue_closing_note` is the one closing-note
path, read from the kind's `closing_note` policy. `thread::Link` is the `link`
event a thread leaves on its parent and heads its own stream with, written by
`Room::write_link` under one id and rewritten as the thread goes; `Link::read`
also reads the old `side` and `subagent` markers, and `Link::wire` turns a link
back into one on its way to a client. A delivery carries `from:
{thread, kind, request}` (`DeliveryFrom`).

Where this differs from the design above:

- **The link is the stored model; clients are sent the old markers.** Phones on
  current builds read `side` and `subagent` and nothing else of a thread, so
  `Link::wire` (a tape's snapshot, its pages and live events, and a thread's own
  stream) sends the shape each kind has always had. A wire change would have
  left those phones with no marker at all. The contract has no `link`; it moves
  with the wire in phase 8, when `thread.*` is added and the markers can go.
- **A link's fields are the design's plus what the marker held.** `thread` and
  `kind` of the design are `thread` and `threadKind`, since the event's own
  `kind` is `link`. `state` is `live`, `parked` or `closed`, `end` says how, and
  `outcome` is the one line. The saved session (`sessionId`, `backendId`), the
  closing `note` and a run's `elapsedMs` ride on it, because the thread's own
  record is read from it.
- **An old marker keeps its id.** A thread started before links is rewritten as
  a link under the id it has (`side:<id>`, `subagent:<id>`), so it is replaced in
  place. New threads are `link:<kind>:<key>`.
- **`xthread:` and `exchange-paused:` are untouched** (phase 6).
- **No id-prefix parsing could be removed.** The kinds ported so far, sides and
  runs, deliver nothing into a DM by id; every producer of `voice:`,
  `handoff:`, `exchange-result:` and `human-answer:` is a kind ported in phases
  5 to 7. The delivery is written with `from` now (`delivery_from`, for the
  pair's and the person's answers), so those reads move onto it with their
  producers. `run_turns` says so where it reads the ids.
- **`recover_exchanges` is started with the sweep, not called from the settle.**
  It is async and waits the first sweep's delay for the rooms to come up;
  `reconcile_exchanges` is called from the settle.
- **The live handles are not one yet.** `LiveSide` and `Running` each hold
  their own driver and lease and close through `end_side` and `Running::settle`.
  They write the same link and take the same note, but a single close waits for
  the DM and pairs to have handles of their own (phases 6 and 7).
- **A run in the sweep is listed, not handled.** The sweep walks the runs on the
  roster and finds `Idle::Never`. A call's row (`Close` at ten minutes) is set,
  and the voice's own clock still ends a call, until phase 5.
- **`Room::me`** is the room's own weak handle, replacing the one `Sides` kept
  for the closing note.

**Phase 5.** A direct call (one with a teammate) is a thread of kind `Call`.
`ThreadId::stream()` is `StreamId::Call`, `calls/<id>.jsonl`, and
`Threads::write` takes it. `voice/record.rs` is the call's writer: the call
pushes each person, voice and relayed line (`relayed: true`) on a channel and a
task of its own appends them in order, so nothing on the live path waits on a
file or the index. The same task writes the call's link (`Room::call_began`,
`call_ended`, which also queue the closing note) with its `outcome` ("Hung up",
"Went quiet", ...) and its `end`. `search_teammate` indexes a call's stream
with side and run lines, named `call:<id>`. `Exchange::from_thread` rebuilds the
voice's memory from the stream under the exchange's own caps when a call is
picked up again under an id it had. The call's quiet clock is gone from the
voice: `Calls::quiet` reports how long each call has been quiet, and
`Room::sweep` applies the `Call` policy's `Idle::Close(QUIET_MS)`. `settle`
closes a call the last process left live as stopped, at the time of its last
line. A direct call's turn names its thread (`Wired::from`, `Origin::from`), so
the `voice:` id is read only for the desk's calls.

Where this differs from the design above:

- **Desk calls stay on the `voice-dispatcher` tape.** They name no teammate, so
  they have no DM to hang off, and the dispatcher reads that tape's tail as what
  was said on the desk lately, across calls. A thread per desk call would take
  that context away. They are indexed as before, still end in the sweep, and
  have no thread or marker.
- **The client marker is `call`, additive.** `Link::wire` sends a call's link as
  `TranscriptEvent::Call` (`callId`, `title`, `status`, `durationMs`,
  `outcome`). The desktop draws a quiet line, "Call · 4 min · Hung up". It has
  no Open yet: reading a closed call in the pane waits for the thread view
  (phase 9). The phone drops transcript kinds it does not know (its tape filter
  and renderer have no case for them), so no capability gate was needed.
- **Memory is rebuilt only for a call taken up again.** A call id is minted by
  the client for each call, so this matters for a call re-opened under its id;
  the rest of the call's lines are read from the thread by nothing else yet.
- **A call that settles has no `parked` state.** Its restart row is `Close`
  (stopped), as designed; there is nothing to resume.

**Phase 6.** Work threads (BRO-201).

- **One kind.** `ThreadKind::Side` is the work thread: the name stays as the
  stored and wire name so every old link, marker and phone build keeps working.
  The policy's tool variant is `Tools::Work`. A `Link` gained an optional
  `opener` (`openerId`/`openerName`); the `side` marker and `SideThreadSummary`
  gained an additive `openedBy`. A teammate's handoff opens a work thread on the
  target (`dispatch_handoff`, `Sides::bring_up`) and never lands in the target's
  DM; the brief arrives as a `Delivery` with cause handoff whose
  `DeliveryFrom` is the sender's DM and the request. `write_link` also writes a
  copy of the link on the opener's tape (`<id>@<opener>`), so both DMs carry a
  marker with title, state and outcome.
- **Result.** When the handoff turn ends and no human gate is open, the thread
  closes (`End::Agent` or `End::Failed`) and the result is delivered to the
  sender's DM, or to the sender's work thread when it was sent from one
  (`reply_thread`), from the work thread (`delivery_source`). The `handoff:` and
  `exchange-result:` ids are only idempotency keys; the turn loop reads
  `Wired::from`. `Stop` on the exchange cancels and closes the thread.
- **Tools.** The same teammate tools as the main session, bar `new_chapter` and
  `resume_chapter`, plus `archive_thread`. `request_human` raises its card in
  the thread, pushed with the thread's id; `send_file`, `generate_image` and
  `message_teammate` post and reply there. Schedules and loops stay with the
  persona and wake the DM.
- **Cap.** `MAX_LIVE` is 3 live work threads per teammate, operator- and
  teammate-opened together; parked ones do not count. At the cap the idlest
  non-working thread parks. A handoff arriving at a teammate whose places are
  all mid-turn stays `Queued` (`Sides::has_room`, polled by the exchange
  worker): nothing is interrupted or refused, and a person's thread is never
  parked for a colleague's request while it is working.
- **Computer lease.** `computer/gate.rs` puts a loopback proxy in front of the
  teammate's computer MCP URL, one per agent. `tools/call` takes the
  teammate's lease (`Leases::take`), waiting up to 20 s and otherwise telling the
  caller who has it; `tools/list` and `initialize` pass through. A thread keeps
  it until its turn ends, until it has been idle 30 s with no call in flight, or
  until its capability lease is revoked (the gate then closes). The main DM and
  peer sessions take part under the keys `dm` and `pair:<key>`. It never widens a
  grant: the gate only forwards to the URL the thread was already granted.
- **Asks.** `Threads::write` takes Pair. A peer card is pushed and counted for
  the answering teammate (`threads_waiting`), and `peers.answer_permission`
  (owner seat only) answers it while the turn is behind it. Pair lines are
  indexed under both teammates, tagged `pair:<key>`, and a rebuild finds them
  from the `peer` markers on the tape. `sweep_peers` is a Pair row of
  `Room::sweep`.

Where this differs from the design above:

- **Read receipts are not in the shared unread.** They are a per-message state
  on the pair stream the peer pane draws; folding them in would change the
  stream's shape for no new behaviour. They stay with `peers.mark_read`.
- **Queue, not park, for a handoff at a full teammate.** Parking a person's
  working thread for a colleague would break the first rule of the phase.
- **The old `Pair (handoff)` kind is gone.** Pair now means asks only, with the
  exchange bookkeeping (`exchange_pair` phases, `EXCHANGE_CAP`, first-contact
  approval) as pair policy. A handoff saved before this phase (no `thread`)
  that was waiting on a human is failed back to the sender on startup.
- **A result for a work-thread sender returns to that thread** and not the DM.

**Phase 7.** The DM is a thread (BRO-202).

- **One builder.** `start_now` builds through `Room::thread_agent` with the Dm
  policy and keeps the per-teammate start gate. The Dm row says what is its
  own: `Resume::Checkpoint` (the teammate's own checkpoint is the session it
  resumes, and the only one it writes), `Seed::chapters` (the open chapter's
  lines are the agent's history and the wake block is in its preamble),
  `Computer::Download` (a start does not wait for an image that is downloading;
  the agent is told, and the computer joins after the turn), and the folder and
  skills it writes. `thread_agent` also subscribes to the driver's info,
  unprompted and subagent streams before it starts, for the DM only, and returns
  them with the driver's whole start report (`ThreadAgent::started`). `start_now`
  is the room's part: the gate, the roster, the `Session` it publishes, the
  watches and the chapter it opens. The computer is held under `Driving` key `dm`
  (`agent::lease_key`, now the one place a thread's key is made).
- **One loop.** `Room::run_queue` (`session/turns.rs`) replaces `run_turns` and
  `run_side_turns`. A kind is an `Occupant`: `Session` in `session/dm.rs`,
  `LiveSide` in `session/sides.rs`. The turn is one `Threads::drive`, over
  `runner::drive_updates`, which tells a `Witness` what each update came to.
  `Threads::turn` (a work thread and a run) uses the `Told` witness; the DM's is
  `Heard`, which carries what is the DM's alone: the stamps of the funnel, read
  receipts, the checkpoint, the reply to a call and to the phone, push for a
  permission card, and steering a line into a turn in flight.
- **Chapters stay DM policy.** Begin, close and resume are unchanged. The idle
  sweep is the Dm row of `Room::sweep` (`Policy::of(Dm).idle == Idle::Chapters`),
  the one row that is awaited. Checkpoints are still stamped on the chapter
  marker, and `resume_chapter` still nudges.
- **Provenance is typed to the end.** A queued line carries `voice` (the call it
  was said on) and `spoken`, so the loop reads no ids. The producers of
  `human-answer:`, `handoff:` and `exchange-result:` ids already set
  `DeliveryFrom`; those ids are only idempotency keys now. The one place an id is
  still parsed is `stopping.rs`, which restores a line kept across a stop.

Where this differs from the design above:

- **The DM's and a work thread's lines are still two types.** `Wired` carries
  what a schedule, a call and a delivery stamp on a line, and a work thread's
  `Line` carries a handoff. The loop is generic over the line, so one type was
  not needed to share it, and merging them would have put a schedule's authority
  on every thread's line.
- **The DM's witness is its own.** The tape's write door stays
  `Room::write_value`: `Threads::write` hands a DM to it and does no push, call
  mirror or `wake_roster` for a DM. Those are the turn's (`Heard`), because the
  DM's push is fed by an update and not by the stored line, and a `wake_roster`
  on every DM card would add roster events no client expects.
- **`Idle::Chapters` is awaited.** Closing a chapter waits on its note, so
  `Room::sweep` does the Dm row on its own and calls `sweep_threads` for the
  others, which it still does synchronously.
- **`xthread:` and `exchange-paused:` stay as they are.** The first is the
  pair's `peer` marker, with an exchange count of its own that a client reads;
  the second is a card, not a link. Writing either through `write_link` would
  change a wire shape, which is phase 8's.
- **The two live handles are not one.** `Session` and `LiveSide` each hold their
  own driver and lease and close through their own door; `Occupant` is the
  seam where they meet, not a shared handle.
- **A peer session is still built by `peer_session`** and driven by `drive`.
  Its policy row is set; it has a queue of one and no person, so it did not need
  the loop.

Notes for phase 8:

- `thread.list | open | prompt | cancel | park | close | continue | answer` can
  sit over `Occupant` and `ThreadStore`: every command reads `ThreadId` and the
  live handle (`Sides::get`, `Room::session`) or the record. `thread.prompt` to a
  DM is `Room::prompt`, to a work thread `say_in_side`; they already end in the
  same loop.
- One `ThreadDelta` replaces `AgentDelta`/`ThoughtDelta` and the side pair. The
  DM's deltas come from `Heard::delta` and a work thread's from `Told::delta`
  (`turns.rs::delta_of`); a run's are not sent. Both already know their
  `ThreadId`, so the new delta is one `match` there.
- `{thread: id}` has one stream per thread to serve: `Tape`, `Side`, `Pair`,
  `Run` and `Call` are all `ThreadId::stream()`. The links on a stream must
  still go through `Link::wire` for phones without the `threads2` capability,
  and that capability is the gate for sending `link` itself.
- The old commands (`session.prompt`, `side.*`, `peers.*`) stay as aliases. The
  DM's `session.start` is the room's start gate, not a thread command: a thread
  is started by being spoken in.

## Today

The map this design replaces, as of `qa/sides-voice` (main, plus side
threads, resumable side threads, and the call voice with memory), updated for
the phases built so far.

| Concern | DM | Side | Pair | Call | Run |
| --- | --- | --- | --- | --- | --- |
| Stream | `Tape` | `Side` | `Pair(key)` + sidecar + `exchange_pair` on Room | `Call`; desk calls on a `voice-dispatcher` tape | `Run` |
| Write helper | `write_value` | `Threads::write` | `Threads::write` (`write_thread`) | `Threads::write` (desk calls: `voice_record`) | `Threads::write` |
| Agent builder | `Room::thread_agent` (from `start_now`) | `Room::thread_agent` (from `bring_up`) | `peer_session` | dispatcher front | `Room::thread_agent` (from `run_to_end`) |
| Turn loop | `Room::run_queue` (`Session`) | `Room::run_queue` (`LiveSide`) | `drive` | voice `run` | `Threads::turn` |
| Resume | persona checkpoints | marker `sessionId` | reseed from thread | `Exchange::from_thread` | none |
| Idle | `Room::sweep` (chapters) | `Room::sweep` (3 h) | `Room::sweep` (10 min) | `Room::sweep` (10 min) | `Room::sweep` (none) |
| Restart | `Room::settle` | `Room::settle` | `Room::settle` + `reconcile_exchanges`, `recover_exchanges` | `Room::settle` (closed) | `Room::settle` |
| Marker | — | link | `xthread:`, `exchange-paused:` | link (`call`); `voice:` id for desk calls | link |
| Cards answered by | `session.answer_permission` | `side.answer_permission` | `peers.answer_permission` | never | nobody (expired) |
| Push / waiting | yes | yes | yes | fallback push | no |
| Search | indexed | indexed | indexed | indexed | indexed |
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
