# ADR 0008: One agent thread layer; modalities attach to it, and the chat is the log of truth

- Status: Accepted by George, 2026-10-19 (task-215). Nothing here is built yet.
- Date: 2026-10-19
- Builds on: [ADR 0001](0001-threads-are-a-primitive.md), whose runtime
  this finishes for pairs and calls
- Would supersede, once its slices land: [ADR 0005](0005-front-is-a-full-agent.md)
  (the live voice is a full agent on the call), and with it what
  [ADR 0004](0004-live-calls-get-a-voice-front.md) still holds for direct
  calls; the "Quiet" rule in [sessions.md](../sessions.md#quiet) that only
  a Hotline Agent quiet run may escalate
- Design: [threads.md](../threads.md), [sessions.md](../sessions.md),
  [voice-calls.md](../voice-calls.md), [security.md](../security.md)
- Tracking: epic-20; it bears on epic-19 (task-213, task-214) and epic-17

## Context

George, 2026-10-19 (epic-20): an agent is a thread. A text DM, a side
thread, a subagent, a scheduled or quiet run, a pair thread and a voice call
are each the same agent thread with a different modality attached, and
there should be one agent thread layer they attach to rather than a path
per feature. Voice is a modality, not a separate agent: on a call the
teammate itself is the full agent, with its tools, with speech in and
speech out. Any thread on any driver can reach the main chat through one
escalation primitive, today's `tell_person` adopted and renamed if a better
name fits. The person always falls back to the chat, and the system is
built that way: on a call, a doc, a link, a file or a summary is sent to
the chat, not read out.

ADR 0001 made the thread the primitive and phases 1 to 9 of
[threads.md](../threads.md) put the DM, work threads and runs on one write
path, one builder and one loop. Three things it left are what this ADR is
about: the pair and the call still have paths of their own, the call's agent
is a second agent (the voice front) rather than the teammate, and a thread
other than a quiet run has no door to the chat.

Line citations are to `release/0.41.0` at `419bdac5`, before task-212 moves
the voice engines.

## Today

### The kinds, and the path each takes

| | DM | Work (side) thread | Subagent run | Scheduled run | Pair thread | Voice call |
|---|---|---|---|---|---|---|
| Identity | `ThreadId::dm`, the `Tape` | `ThreadId::side`, `sides/<id>.jsonl` | `ThreadId::run`, `runs/<id>.jsonl` | none: a turn of the DM, its user line stamped `scheduled` (`session/prompt.rs:777-791`) | `ThreadId::pair`, `threads/<key>.jsonl` | `ThreadId::call`, `calls/<id>.jsonl`; a desk call is on the `voice-dispatcher` tape, no thread |
| Built by | `Room::thread_agent` from `start_now` (`session/start.rs:78`) | `Room::thread_agent` from `bring_up`/`launch` (`session/sides.rs:650`, `:727`) | `Room::thread_agent` from `run_to_end` (`session/runner.rs:389`) | the DM's session | `peer_session` (`session/peers/sessions.rs:308`), its own builder | nothing: the call has no agent (`Tools::None`, `thread/policy.rs:216-224`); a realtime front is opened by `run_front` (`voice/duplex.rs:323`) |
| Driven by | `Room::run_queue` with `Session` (`session/turns.rs:285`, `session/dm.rs`) | `Room::run_queue` with `LiveSide` | one `Threads::turn` (`session/turns.rs:181`) | the DM's queue | `runner::drive` (`session/runner.rs:91`) | the front's socket; heavy work relayed into the DM by `hand_off` (`voice/turn.rs:379`) |
| Whose agent | the teammate's main session, resumed from its checkpoint | a fresh agent of the teammate, seeded with the DM's tail, resumed by its binding | a fresh worker with no conversation | the teammate's main session | a fresh session of the target, checkpoint stripped | a realtime model speaking as the teammate, with the teammate's desk-side reads (`voice/front_tools.rs`) |
| Lease | `Same` | `Independent` epoch | `Scoped` child, revoked when the run settles | the DM's | caller's `scoped()` plus the target's | the teammate's current lease, checked per tool (`Room::front_reach`, `session/start.rs:50`) |
| Drivers | both | both | Hotline Agent only (`session/runner.rs:398-400`); an ACP harness's own subagents are mirrored as runs | both | both | the front is the provider's; work goes to whichever driver the DM runs |
| How its words reach the person | the chat | its own thread, opened from a link on the DM | a report to the teammate's turn as a job result; the person can open the transcript | loud: the chat; quiet: demoted to thinking by kind (`session/quiet.rs:55`, `session/prompt.rs:767`) | a delivery to the asker's DM or work thread; the person reads along | spoken; the call thread keeps it; the DM gets a summary at hang-up and every 5 minutes, and `post_to_chat` (`Room::call_posted`, `session.rs:1015`) |
| May wake the person | a reply pushes (`send_glance`, `session.rs:973`) | cards push; replies do not | never (`Answer::Nobody`) | loud: as the DM; quiet: only by escalating | cards push | the person is on the call; cards are signalled to it |
| Escalation | — | none | none, by design | quiet runs on Hotline Agent only | none | `post_to_chat`, the front's tool |

### Escalation, exactly

- **The trait.** `Escalate::escalate(&self, text)` (`driver.rs:205-214`),
  armed per turn by `Driver::escalate_next` (`driver.rs:326-330`). The
  default is a no-op; only Hotline Agent implements it (`driver/rig.rs:414`),
  so an ACP teammate's quiet run cannot escalate.
- **Who.** `Session::begin` arms a fresh `Escalation` only for a turn whose
  `scheduled.quiet` is true, and clears it on every other prompted turn
  (`session/dm.rs:125-145`). Work threads, pairs, runs and fronts never arm
  it. `tell_person` is not on the MCP server (`mcp/server.rs:84-104`); it is
  a Rig tool added for the one prompt (`driver/rig/tool_output.rs:17-63`,
  `driver/rig.rs:614-617`).
- **How often.** Once per run: a second call is refused
  (`session/escalation.rs:36-47`).
- **What it does.** Nothing is posted by the call. When the quiet turn ends,
  `Room::escalate` (`session/dm.rs:426-454`) writes the note as a `user`
  line stamped with the job (not quiet), and queues at the front a turn that
  tells the teammate "Tell them now, in your own words"
  (`session/escalation.rs:52-57`). The teammate's loud reply is what the
  person sees.
- **How it notifies.** As any reply: the rail's unread, the desktop toast
  (`ui/src/notify.ts`), and a push (`send_glance`), held while the person is
  at the desk. The trait's doc comment says the room posts the text itself,
  which is no longer what it does.
- **Tests.** Unit tests only, in `session/tests/scheduled.rs`; no headless
  harness covers it.

The other doors into the DM are each their own: `call_posted` writes an
agent message directly, with a `call:<id>:` id and a "From the call: " text
prefix, pushes nothing, and queues the text for the session's next turn
(`pending_calls`, `session/prompt.rs:313`); a pair's result and a handoff's
are deliveries (`deliver_identified`, `session/prompt.rs:183`); a work
thread's end rewrites its link on the DM. A file sent from a work thread
lands in that thread, not the chat (`session/files.rs`), and a file sent from
a quiet run is refused (`session/files.rs:35`, `:76-78`).

## Decision

### 0. Names (George, 2026-10-19)

A teammate is one agent with many arms. Everything it does, wherever it does
it, funnels back to the person; that is what makes it an agent rather than a
set of features. The names below are the ones code, docs and the window use,
and every future feature is placed among them.

| Term | What it is |
|---|---|
| **Teammate** | The whole agent: its home agent, its arms and its abilities. |
| **Home agent**, **home thread** | The teammate's main session, and its conversation with the person (today's DM). "Core" is not used: it collides with `hotline-core` and the core cycle. |
| **Arm** | The teammate working outside the home thread: a work thread (today's side thread), a subagent, a job, a call, a pair (with another teammate). An arm runs on the home agent's model and workspace, with a lease derived from the home thread's and never wider. |
| **Ability** | What any arm may use: tools, the computer, skills, MCP servers, as its lease allows. |

**A subagent is a clone with a focus** (George, 2026-10-19). The picture is
the home agent saying "I need to be in three places at once" and spawning
clones of itself, each with one focus. A clone is the same teammate, not a
lesser one: the home agent's model, workspace and abilities, under the same
permission set (its lease is the home thread's, narrowed only if the home
agent narrows it for the task). It does not start with the home thread
poured in, for context and above all for focus: it starts from its brief,
and reaches back on demand, as the home agent would, into the home thread's
history, chapters and other threads with the same tools (`search_thread`,
`list_chapters`, the thread readers), within the same lease. What it learns
along the way is its own context, kept on its thread.

### 1. The agent thread

An **agent thread** is a thread (ADR 0001) with an agent bound to it. It is
four things, each already a policy row in `thread/policy.rs`:

- **its agent**: the teammate's, on the teammate's driver, built by
  `Room::thread_agent`, resumed by the thread's binding or seeded from its
  own stream;
- **its lease**: derived from its parent by `lease_of`, never wider;
- **its stream**: `ThreadId::stream()`, written only through
  `Threads::write`;
- **its context**: the kind's `Seed` (the parent's tail, its own history,
  the DM's chapters).

What every modality shares is the path: one builder (`Room::thread_agent`),
one turn (`Threads::drive`, under `Room::run_queue` for any thread with a
queue), one write door (`Threads::write`), one lifecycle (`Room::sweep`,
`Room::settle`, the closing note, the link), and one door to the chat
(below). The **kind** stays what it is today: which stream, which parent,
whose agent and which lease. Nothing is renamed and no file moves.

Two kinds are off this path today and come onto it: the pair (built by
`peer_session`, driven by `runner::drive`) and the call (no agent). After
that, any thread with an agent is built and driven the same way, and a
seventh kind would be a policy row and a modality, not a subsystem.

### 2. A modality

A **modality** is what is attached to a thread's turns: where the line
comes from, where the agent's words go, whether the person sees them as
they are said, and whether they may wake the person. It is not a kind: a
quiet run is a modality on the DM's turns, and a call is a thread whose
turns carry the voice modality while the call is up. In code it is what
the turn's line and witness already carry (`Wired.scheduled`, `Wired.voice`,
`Wired.spoken`, the quiet window, `Heard` and `Told`); this ADR adds one
output, *spoken*, to a thread other than the DM, and no new type.

| Modality | Thread it attaches to | Input | Output | Words visible | May wake | What stays its own |
|---|---|---|---|---|---|---|
| Text DM | the DM | the person's typed line; deliveries | the chat | yes | a reply pushes | chapters, checkpoint, running processes |
| Work (side) thread | a work thread | the person's line, or a colleague's handoff | the thread | yes, in the thread | cards; posts to the chat | the cap of three, the computer lease, archive, the handoff's result |
| Subagent | a run | the parent's task | a report to the parent's turn | read-only transcript | never | job control (`inspect_job`, `wait_jobs`), no conversation |
| Scheduled run, loud | the DM | the job's prompt | the chat | yes | as a reply | the job stamp, Background work |
| Scheduled run, quiet | the DM | the job's prompt | thinking, by kind | no | only by posting | the quiet window and its 30-minute cap |
| Pair thread | a pair | a colleague's ask | a delivery to the asker | read along | cards; posts to the target's chat | first contact, the exchange cap of 12 |
| Voice call | a call thread | speech, transcribed | speech, and the call thread | the call card and its thread; the chat by summary and posts | no: the person is on the call; cards are said on it | the speech engines, barge-in, the Voice budget, the spoken contract |

The rule a modality cannot break: it narrows, never widens. It picks what
the person sees and hears, never what the agent can reach; reach is the
lease's.

### 3. Escalation: one door to the chat, `post_to_chat`

**The name.** `post_to_chat`. It names where the words go, which is the
point of the design: the chat is the log of truth, and posting to it is not
always an alarm (a call's summary wakes nobody). `tell_person` names an
intent that only fits a quiet run, and `post_to_chat` is already the name
the voice front's model knows. One tool, one name.

**The primitive.** In the core, `Room::post(teammate, from: ThreadId, post)`
writes one agent message to the teammate's DM. It is the only way anything
not said in the DM itself reaches it: the tool, a file or image sent from
any thread, a call's summary. It:

- writes an `agent` event, as the teammate, stamped with where it came
  from: an optional `from` (`DeliveryFrom`: the thread and its kind) and,
  for a scheduled turn, the job's `scheduled` stamp. Both are additive
  optional fields on `TranscriptEvent::Agent`; the reference tree's
  importer must read an agent event that carries them, and design.md says
  so when slice 1 lands (AGENTS.md, rule 4). They replace the `call:<id>:`
  id and the "From the call: " text prefix; a client draws "From the call",
  "From a scheduled job", "From *thread title*" from `from`;
- lights the rail, raises the desktop toast and pushes as a reply does,
  except when `from` is a call that is live, since the person is on it;
- reaches the home agent by who asked for the work (George, 2026-10-19):
  - **the person asked** (they scheduled the job, placed the call, opened
    the work thread): the post is for the person. The home agent is not
    woken; the post is handed to its live session ahead of its next line
    (`pending_calls` generalised), so it knows what its arms said and a
    typed follow-up does not start cold; a session that starts later reads
    it from the tape;
  - **the home agent asked** (a subagent it launched, a check it scheduled
    for its own work): the result wakes the home agent once, as a subagent's
    report does today, and the home agent decides what the person hears;
  - **an arm needs the home agent to act**, not only the person to know
    (the build broke under a refactor the home agent is in the middle of):
    `post_to_chat(text, wake: true)` posts and wakes it once. The handler
    allows one wake per turn, as one post per turn;
- never opens or obeys a quiet window: it is the one door out of one.

**The tool.** `post_to_chat(text)` is one of Hotline's own tools on the
desk's MCP server (`mcp/server.rs`), so an ACP child reaches it over the
server it already has, and Hotline Agent through the same handler in-process
(`as_dynamic`). It is listed for every session, because an ACP child's tool
list is fixed when its session starts; whether a call may post is decided by
the handler, outside the model:

| Safeguard | Today (`tell_person`) | `post_to_chat` |
|---|---|---|
| Who may | a quiet scheduled turn on Hotline Agent | any agent thread of the teammate, on either driver: a quiet turn, a work thread, a pair (to the target's own chat), a call. Refused on a loud DM turn ("your reply is already the chat") and to a subagent the home agent launched; a job (§7) may post |
| Where | the teammate's DM | the teammate's own DM only; never another teammate's |
| How often | once per run | once per turn of the thread; a second call in the turn is refused in the same sentence as today. A call's summary is the core's, not a turn's, and does not count |
| Authority | the turn it was armed for | the thread's capability lease, checked when the call lands, as `post_to_chat` does now; a revoked lease refuses |
| Stamp | a `user` line stamped with the job, then the teammate's reply | one `agent` message, `from` the thread, plus the job's stamp on a scheduled turn |
| Notify | as the follow-up reply | as a reply, except to the person on a live call |
| Size | free | clipped at 2,000 characters, as the front's is now |

What changes for a quiet run: the note is posted as the teammate wrote it,
rather than re-said by a second turn. The quiet turn's agent is the
teammate's own session, so the words were already its own, and the second
turn cost a model call and could wait behind the person's queued lines. The
tool's description says to write it as the message to the person.

A subagent the home agent launched is the one arm that may not post. It
reports to the home agent, which asked for it, and the home agent decides
what the person hears, as sessions.md has it now. A job (§7) is a subagent
the person asked for, and posts like any other arm. An ACP harness's own subagents may share the parent
session's MCP server; the handler cannot tell them from the parent, so a
post from one is the parent thread's and counts as its one.

This is a capability change and follows [security.md](../security.md):
default on for every agent thread but a run; the grant source is the
thread's kind and the turn's modality, read by the handler; enforcement in
the handler and `Room::post`; proved allowed and refused (loud DM turn, a
run, a second post, a revoked lease, another teammate's chat) through the
real handler on both drivers; residual risk, a teammate on a long work
thread posting once a turn.

### 4. The chat as log of truth

**Always reaches the chat**, through `Room::post`:

- an escalation, from any thread on any driver;
- a file, image or document an agent sends, from any thread: `send_file`
  and `generate_image` post to the DM, stamped `from` the thread, and the
  thread keeps a line naming it. In a quiet run a file is a post and counts
  as the run's one, which removes today's refusal and its cleanup;
- on a call, any reply whose written version carries what speech cannot (a
  link, a code block, a table, an attachment): the core posts that written
  version, decided by what the text contains, not by asking the model to
  remember. The spoken contract already asks for both versions;
- a call's summary at hang-up (section 5);
- a work thread's result and a pair's answer, as today: a link and a
  delivery.

**Never reaches the chat as a message:**

- a quiet run's words: they stay thinking on the DM's tape, as now, and are
  never a preview, an unread or a push;
- a call's small talk: it is on the call's own thread, which the call card
  and the thread view show;
- a subagent's working: its transcript is its run's; its report is the
  teammate's to pass on.

A link said in a visible work thread stays in the thread: the person read it
there, and the thread is linked from the chat. An agent that means the
person to keep it posts it.

### 5. Voice under this design

**A call is a thread whose agent is the teammate.** The `Call` policy row
becomes a work thread's in what it reaches (the teammate's tools, plus
`post_to_chat`, less the chapter moves; `Lease::Independent`; the computer
through the lease; seeded with the DM's tail; resumed by its binding) and
keeps its own lifecycle (parked 30 seconds after hang-up once no turn is in flight, one per teammate).
It is built by `Room::thread_agent` and driven by `Room::run_queue`, as a
work thread is. On a harness teammate it is an ACP session of the harness's
own, so Claude Code's edits and Codex's shell are on the call; ADR 0005's
open question about lending a harness's tools to a call goes away.

**Speech in, speech out.** The call's ear transcribes (the `Speech`
engines, or the device's own text, `voice.text`) and each utterance is a
line on the call thread's queue; a second utterance steers a turn in
flight, as it does on the DM. The call's mouth says the reply's spoken
version as it streams, cleaned for speech (`spoken::CONTRACT`,
`speech_text`), and the turn writes both versions to the call's thread. The
first thing said is the agent's own first sentence, before its tools, which
is where epic-17's acknowledgements ("Let me look at the voice code") come
from; a soft sound covers a turn that says nothing first.

**Hanging up detaches the modality; it does not end the turn.** A turn in
flight at hang-up finishes, and its reply is posted to the chat instead of
said, which replaces today's follow-up of an unanswered question
(`duplex.rs:292`, `spoken::ENDED_NOTE`). When the call is over, 30 seconds after hang-up once
no turn is in flight (a turn still running at hang-up finishes first), its
closing note, the same summariser a work thread's close uses, is posted to
the chat as the call's summary, and the thread parks (George, 2026-10-19: ten
minutes was too long). Parking closes nothing: the thread can still be
continued by text or picked up by a new call, and if it is, the next hang-up
posts a note covering what is new since the last one. The five-minute
summaries and the front's mechanical `Digest` (`voice/record.rs:116-207`)
go: on a long call the person sees the transcript on the call card, and what
matters is posted as it happens.

**The live fronts.** A realtime front stops being an agent: its prompt
(`front_prompt.rs`), its registry (`front_tools.rs`, `FRONT_TOOLS`,
`for_front`, `Room::front_reach`), `send_to_teammate` and `remain_silent`,
the call assistant's delegation loop (`dispatcher::answer_delegation`) and
the relay into the DM (`hand_off`, `CALL_ORIGIN` on DM turns) go. The
realtime engines are then the ear and mouth or nothing.
**Recommendation:** run the call on the `Speech` engines alone (streaming
transcription and progressive speech already exist on an xAI key, and the
Grok login is a speech choice of its own), because once the agent answers every utterance the
front's latency advantage is gone and a speech-to-speech model told to say a
text verbatim is the one place the person could hear words no agent wrote.
The proving call measures the end of speech to the first audio; if the ear
and mouth, not the agent's turn, cost more than a second there, a realtime
session comes back as ear and mouth only, behind the `Speech` seam, with no
tools and nothing to say of its own. Two things are lost and stated: GPT-Live
on the ChatGPT sign-in has no pipeline equivalent, and small talk is
answered at an agent's speed. A teammate whose call should be quick runs the
call on a fast model, which is the teammate's model setting, not a voice
one.

**A call goes on as text, and a new call can pick it up (George, 2026-10-19).**
A required feature, not a later UI choice. A call thread keeps its binding
when the person hangs up, so it stays an open thread: back at the desk they
keep talking to the same agent in it by text, with the call's whole context,
and from there they can start another call on that same thread, attaching
the voice modality to it again. Hanging up detaches the voice and closes
nothing. The window and the phone both offer it from the call's thread.

**A call can start on any thread (George, 2026-10-19).** The same feature
in reverse: a call can be placed on a thread already under way, a work
thread or an earlier call, and the voice modality attaches to that thread
and its agent instead of opening a new call thread. Whatever is said lands
in that thread, and posts go to the chat as from any call. A call on a
thread whose agent is mid-turn waits or interrupts as a typed line would.

**Desk calls.** epic-17 makes a desk call a call to the team lead. Under
this design that is a call thread on the lead, so the dispatcher's desk role,
the `voice-dispatcher` tape and its `desk_command` tool (`voice/dispatcher.rs:101`,
`:989`), which runs the wire's command table, go with epic-17's routing
task. Until a lead exists, desk calls stay as they are.

**epic-19 and task-213.** The engines' move (task-212) is unaffected: the
`Speech` engines are the modality's ear and mouth. task-213 inventories the
room's side of a call today, and its recommendation should be sized against
this target, where that side is: open and close a call thread, queue a heard
line on it, take the agent's spoken stream back, post to the chat, and be
told of a card. That is a handful of methods, and the reach George's test
counts against (`front_reach`, `start` and `prompt` into the DM,
`call_posted`, `voice_spoken`, and the dispatcher's command table) is what
slices 5 to 7 delete. task-214 should move the calls after slice 6, not
before: moving the fronts into `hotline-voice` and then deleting them there
is two changes where one would do.

### 6. Migration

Each slice lands alone, keeps every test green, and says what it removes.

1. **`Room::post`, one door to the chat.** `call_posted`, the quiet run's
   follow-up and `send_file` from a thread write through it; the agent event
   gains `from` and `scheduled`; a quiet run's note is posted as written.
   *Removes* `Room::escalate`'s follow-up turn, `escalation::follow_up`,
   `pending_calls` as its own field, the `call:<id>:` id and "From the
   call: " prefix. *Proved by* a headless harness: a quiet run that posts,
   one that does not, and a call's post, each read off the wire.
2. **`post_to_chat` on the desk's MCP server, for both drivers.** The
   handler's gate (kind, modality, once a turn, lease). *Removes* the
   `Escalate` trait, `Driver::escalate_next`, the Rig-only `tell_person`
   (`driver/rig/tool_output.rs`) and the per-turn arming in
   `Session::begin`. *Proved by* the security rows above, an ACP quiet run
   that escalates, and the stale "Sixteen tools" and `Escalate` comments
   corrected.
3. **Artifacts land in the chat.** `send_file` and `generate_image` from a
   work thread, a pair or a call post to the DM; a quiet run's file is its
   post. *Removes* the quiet refusal and its kept-copy cleanup in
   `session/files.rs`.
4. **Pairs on the one path.** `peer_session` builds through
   `Room::thread_agent` and drives through `Threads::drive`. *Removes*
   `peer_session`'s own builder and the pair's use of `runner::drive`, and
   the stale comment in `Policy::of`.
5. **A call has an agent.** The `Call` row as above; a `LiveCall` occupant
   on `Room::run_queue`; the pipeline (Legacy voice) feeds the call thread
   instead of the DM; hang-up detaches; the closing note is posted. Fronts
   still run when Settings picks one. *Removes* the pipeline's relay into
   the DM (`hand_off`, a direct call's `CALL_ORIGIN`, `Origin::ended`,
   `ENDED_NOTE`, `voice_spoken` on DM replies) and the call's `Tools::None`.
6. **The fronts go**, after the proving call on slice 5. *Removes*
   `front.rs`, `front_prompt.rs`, `front_tools.rs`, the front half of
   `duplex.rs`, `Digest`, `answer_delegation`, `Room::front_reach`,
   `FRONT_TOOLS`, and the realtime engines unless the measurement above
   keeps them as ear and mouth. `voice.call_start.mode: "front"` is answered
   without a `front`, which every client already reads as the pipeline, so
   no wire field is removed while older phones send it. ADR 0005 and what
   ADR 0004 held for direct calls are marked superseded.
7. **Desk calls are the lead's** (epic-17's routing task). *Removes* the
   dispatcher's desk role, the `voice-dispatcher` tape and `desk_command`'s
   reach into the wire's command table.

Slices 1 and 2 stand alone and are what George asked for first; 3 and 4 are
independent of voice; 5 to 7 are the voice path, in order.

### 7. Jobs are subagents on a schedule (George, 2026-10-19)

A background job is the teammate doing a recurring task outside the home
thread and reporting back: status when the person wants it, and anything of
significance. It is not a mode of the home thread. So jobs and subagents are
one thing with one schema and one runtime:

| | Trigger | Report |
|---|---|---|
| Subagent | now | to whoever asked (the home agent, usually) |
| One-off job ("do X at 9") | at a time | to whoever asked (the person, usually) |
| Recurring job ("check every 30 min") | every interval | to whoever asked, when something is worth saying |

- **One schema:** the task, the trigger (`now`, `at`, `every`), the report
  policy (who asked; status on a cadence, or only on significance), and an
  optional end (a goal, a date, a count). The `schedule`/`loop` tools and the
  subagent tool write it; today's quiet flag becomes the report policy.
- **Its own thread.** Each job is an arm with a thread under the home thread,
  listed with the person's threads in a Jobs group with its next run. Each
  firing is a turn on that thread, never the home thread, so the home agent's
  context holds none of it. The person can open the thread and talk to it
  ("skip the next check", "also watch CI").
- **Same agent, a clone with a focus** (§0). It runs on the home agent's
  model and workspace with the home thread's permission set, starts from its
  task and running note rather than the home thread, and reaches into the
  home thread's history, chapters and threads with the same tools when the
  work needs it. A slice of this ADR inventories what today's subagent runs
  are given (their lease is scoped narrower than the home thread's today)
  and brings them to this.
- **Parked between firings**, its agent released and reopened from its
  binding at the next one, as a work thread parks.
- **A running note, not a growing context.** Each firing starts from a short
  state note the job rewrites every time, plus its last few firings, so a
  loop that runs for days stays small.
- **A firing still running when the next is due** is skipped, the skip is
  said on the job's thread, and the job suggests an interval that fits the
  time its work takes, as a teammate would. A job that keeps overrunning
  says so to the person once.
- **Ends** when the person cancels it or the job meets its goal and cancels
  itself, with a final post saying what came of it. Its thread stays
  readable.
- **A plain reminder** ("remind me at 9 to call Sam") does no work: it fires
  as one post to the home thread, with no arm.

Migration: one more slice after posting works everywhere (slice 2): the
schema and runtime shared by subagents and jobs, the job thread and its
note, then today's quiet-run path in the home thread removed.

## Consequences

**Better**

- Every thread on either driver can reach the person, through one tool,
  one door and one set of safeguards; an ACP teammate's quiet run is no
  longer silent by accident.
- The chat holds everything an agent gave the person, stamped with where it
  came from, so searching the chat finds a file made on a call or in a work
  thread.
- A call is the teammate: its harness, its tools, its computer; nothing is
  "passed on", and what is said is what an agent wrote.
- The pair and the call stop being special at the builder and the loop, and
  the front's three files and the room's widest reach into voice go.

**Costs and risks**

- Small talk on a call waits for an agent's turn. The proving call is where
  this is judged, and the measurement in section 5 is what would bring a
  realtime ear and mouth back.
- GPT-Live on the ChatGPT sign-in is lost unless a speech route on that
  sign-in exists.
- A teammate on a call and in its DM is two agents of the same teammate at
  once, as a work thread already is; the computer lease serialises them, and
  the posted summary, and the posts handed to the DM's next line, carry
  the call back to it.
- The agent event gains two optional fields, which the reference tree's
  importer must accept.
- Calls written before slice 5 have no binding and cannot be continued;
  they stay readable.

## Open questions

None left open: the last (a quiet job's own thread) is decided in §7.

## Alternatives considered

- **Attach the voice to the DM itself.** One agent, no second context. But
  every utterance and every "mm, yes" becomes a chat line, which is what ADR
  0005 was written to stop, and a call waits behind whatever the DM is
  doing.
- **Keep the front as the agent and give it the harness's tools.** The ACP
  surface to lend a harness's tools to another model does not exist, and two
  agents speaking as one teammate is the split this epic removes.
- **Keep `tell_person` and add `post_to_chat` beside it.** Two doors with
  two sets of safeguards, and a model choosing between them.
- **Keep the follow-up turn for every post.** Each post would cost a DM
  turn and wait behind the person's queue; the posting thread's agent is
  already the teammate.
