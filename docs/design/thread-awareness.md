# Threads as streams of thought: awareness, asides and agent to agent

Agreed with George, 2026-10-30 (the five answers below, as drawn in
`thread-awareness/` frames 00–09). Builds on
[ADR 0001](../adr/0001-threads-are-a-primitive.md) and [threads.md](../threads.md),
which made every conversation a thread with its kind as policy. This document is
about what an agent knows of its own threads and how threads reach each other.
It is the frame for deciding what to take from #203 (Kelviny Moura's handoff
threads).

## The idea

A thread is a stream of thought. An agent is one identity with many of them:
the main conversation with the person, the work it was handed, the questions
between it and its colleagues, its calls, its jobs, its subagents' runs.

Two rules follow:

1. **An agent has peripheral awareness of all its threads.** It always knows
   it has thread history, and which threads are active or recently ended,
   without having to search for them.
2. **Threads inform; they don't report home.** What happens in one thread can
   become context in another, quietly. It never turns into a message in the
   person's chat unless something genuinely needs the person. There is no
   "report each turn to whoever opened it" loop.

Everything here works the same between two agents on one desk and between
agents on linked desks.

## What counts as a thread

### Threads today (one `ThreadKind` each)

| Kind | What it is |
| --- | --- |
| DM | The person and a teammate: its main conversation. |
| Side | A work thread beside the DM, opened by the person or by a teammate's handoff. |
| Pair | Two teammates talking: an ask. |
| Call | A voice call. |
| Run | A subagent's run of one task. |
| Job | A scheduled job, one turn per firing. |

### Thread-shaped, not yet threads

- **Chapters.** Closed stretches of a DM with a closing note. They are the
  agent's past threads in all but name, reached through their own tools
  (`list_chapters`, `resume_chapter`).
- **Ask sessions.** The memoryless copy that answers a Pair today. Its words
  land on the Pair, but the stream of thought that produced them belongs to
  no thread. This is the gap #203 describes.
- **The voice dispatcher.** A hidden DM-style tape behind live voice.
- **Linked-desk traffic.** Cross-desk asks and handoffs travel as deliveries
  with leases and reports, not as a thread both desks can name.
- **Exchange records.** An ask or handoff's lifecycle is tracked beside the
  threads, not on them.
- **Background processes and computer jobs.** A server or watcher left
  running, or a shell job on the teammate's computer: streams of output an
  agent should be peripherally aware of.
- **Cards waiting on the person.** Each sits on the thread that raised it; the
  attention list is a separate view across threads.

### Missing

- **Inbound threads.** Something outside starting a stream for a teammate: a
  webhook, an email, a GitHub event. A quiet job escalating is the only
  version today.
- **Harness subagents.** Claude Code's and Codex's own subagents run inside a
  turn, invisible. Only Hotline Agent subagents become Runs.
- **Multi-party threads.** The person and two or more teammates in one stream
  (threads.md lists this as an open question).
- **Artifacts between agents.** `send_file` reaches only the person. A
  teammate can't hand a colleague a file, on this desk or a linked one.

## 1. Awareness: the thread index

Each agent gets a short, current index of its threads at the start of every
turn, in every thread it runs in. It is not a tool the agent has to think to
call.

- **What's in it:** each thread's kind, key, title, state (working, waiting on
  the person, parked, ended), when it last moved and its last line in a few
  words. Active threads first, then threads that ended in the last day, then a
  count of older ones ("41 older threads; search_thread finds them").
- **Where it comes from:** `ThreadStore`, which already lists every kind.
  Chapters join it as ended DM threads.
- **How big:** capped (about 15 entries and a few hundred tokens), so a busy
  teammate doesn't pay for its history on every turn.
- **What it is not:** a transcript. To read a thread the agent uses
  `read_thread`, as now.

*Done when:* an agent in any thread can name its other active threads and
what they're doing without a tool call, and the index stays within its cap
(test on the built preamble).

## 2. Asides: informing without phoning home

An **aside** is a short line one thread leaves in an agent's peripheral
context. It is the one mechanism every thread kind uses to inform: a
colleague's pair, a handed-over thread finishing, a job's firing, a process
that died, an inbound event.

```
aside { to: agent, from: ThreadId, text, about?: ThreadId, level }
```

- **It lands in the index, not the chat.** The receiving agent sees it at the
  start of its next turn in whichever thread is relevant (`about`, else its
  DM), as "Quill, in pair:quill~pork-chop: the Q3 numbers are in
  /shared/q3.csv". The person's chat shows nothing new.
- **It doesn't wake anyone by default.** It waits for the agent's next turn.
- **Levels**, lowest first:
  - `note`: context only, waits for the next turn.
  - `nudge`: wakes an idle agent once, if the aside is about a thread that is
    active. Counted against the pair's brake, so two agents can't wake each
    other forever.
  - `person`: needs the person. It becomes a card or a line in the DM, as a
    quiet job's escalation does today. Quiet-job escalation becomes this
    level of an aside.
- **The person can see them.** Asides are listed quietly per teammate (a
  "Noted" section in the thread pane or teammate menu), so context never
  flows between agents unseen.
- **Who may aside whom:** the same grants as asking. A teammate may aside a
  colleague it may ask (allowedSenders or a first-contact card, implicit for a
  Whole machine Hotline Agent). Across a link, the linked desk's grant. An
  aside carries the sender's words only, never the person's.
- **Kept and searchable:** written to the sender's thread and the receiver's
  index, so `search_thread` finds it later.

*Done when:* Quill can leave Pork Chop an aside from a pair or a work thread;
it appears in Pork Chop's next turn and in its Noted list, nothing appears in
the person's chat, and a `nudge` wakes an idle Pork Chop once (tests per
level, plus a cross-desk test).

## 3. Agent to agent, local and across desks

- **Threads are addressable everywhere.** A `ThreadId` with an optional desk
  (`{desk, kind, key}`) names a thread on a linked desk the same way as a
  local one. `message_teammate` and asides take one.
- **Artifacts travel in threads.** A message or aside can carry files. On one
  desk the file is copied into the recipient's workspace (an `inbox/` folder
  under it). Across a link it goes over the link, with a size cap. The
  recipient gets the path in its aside or message.
- **One grant model.** Sending a file needs the same permission as asking.
  Nothing new to approve.

*Done when:* "send Quill this file" works on one desk and across a link, the
file lands in Quill's workspace, Quill's next turn knows where (tests), and
an over-cap file is refused with a reason.

## Decisions (George, 2026-10-30)

1. **Asking a busy colleague.** An ask never waits behind a busy main
   conversation. A work thread of the colleague's answers at once, seeded with
   its thread index and recent asides, and leaves an aside on the colleague's
   main conversation saying what it answered on its behalf. When the main
   conversation is idle, it answers itself. An ask that names a thread is
   answered in that thread.
2. **"Aside" is the word.** The person sees a teammate's asides on a
   **Noted** page of its ⋯ card (between Threads and Search), each with where it
   came from, its level (note, nudge, or "to you" in amber) and its time. In a
   conversation an aside is a quiet line with the `(·)` mark, never a bubble.
3. **The index goes to the main conversation, work threads and calls.** Jobs
   and subagent runs get only their parent thread and the asides about them.
4. **Inbound: shape it now, build it later.** Inbound events arrive as
   asides from day one, through a source model with **its own permissions and
   controls** (below). No transport ships until that model is in.
5. **No multi-party threads.** **Loop in…** on a work thread's ⋯ card sends a
   colleague a `nudge` aside linking the thread. The looped-in colleague may
   answer once in that thread, under its face; the thread shows a quiet "Quill
   was looped in" line. A work thread's ⋯ card has Loop in… where a DM has New
   chapter, and End thread where a DM has Remove.

## Inbound sources: permissions and controls

An inbound source (a webhook, a mailbox, a GitHub repository) is a standing
grant, kept on the desk like a folder or an MCP server, never something a
teammate makes for itself.

- **Owned by the person.** Only the owner's desk or phone adds, pauses or
  removes a source. A teammate can propose one (a card), like `add_teammate`.
- **Scoped to teammates.** Each source names which teammates it may aside, and
  the highest level it may use (`note` by default; `nudge` and `person` must be
  allowed per source).
- **Untrusted content.** What a source delivers is data, fenced and labelled
  as coming from outside, never instructions. It can't carry files into a
  workspace unless the source allows it, and never secrets.
- **Rate-limited and quiet by default.** A per-source limit and a brake, the
  same as a pair's, and a mute switch per source.
- **Auditable.** Every inbound aside is kept and searchable, listed on Noted
  with its source, and the source's own page shows what it delivered.

Shape now means: the `Aside` type carries a `source` that is a thread or an
inbound source, the grant record and its checks exist and are tested, and
the Noted page can show a source. No webhook endpoint, mailbox or GitHub
transport is built yet.

## Mapping #203 to this design

#203 is 80 commits by Kelviny Moura on `feature/handoff-threads` (base
`release/0.44.0`). It still merges into main with three conflicts (two test
files and `ui/src/components/Dock.tsx`). Most of it can't be cherry-picked as
is, because the parts are woven through each other, so it is ported phase by
phase onto this design. Work taken unchanged keeps Kelviny's authorship;
adapted work carries `Co-authored-by: Kelviny Moura
<kelvinyhenrique17@gmail.com>`; the 0.46 release notes thank him.

| Group | #203 commits | Under this design |
| --- | --- | --- |
| **G. A rebuild isn't a revocation; lock order** | 1c8267a3, 8246a1b5, 4269984e, d14f90d5 | **Take as is.** A computer joining, a release swap and an MCP sign-in rebuild only the main session; `persona.update`, `settings.update` and `mcp.auth_sign_out` release `policy_update_lock` before awaiting a reattach. Overlaps task-316. |
| **F. Requests end only when someone ends them** | 632ef23d, 543a2a92, 638e4cee, 0a3f6e53, 28d039b0 | **Take as is.** `CapabilityEpoch.authority` beside `generation`: a swap or rebuild advances only the generation. |
| **A. A handoff keeps its thread** | fb5d18c6, 2fcf7fd3, 1d482942, 27b230f1 | **Take.** `message_teammate` returns `thread` and takes it back to continue. This is thread addressing (§3) on one desk. |
| **B. The person talks in a handed-over thread** | f9cdb4e1, 3bed7168, 2a9fbd82, 38a77fdf, f8287f4c, 247e7f7f, 4eebc70b, 118732b2 | **Take**, plus decision 5's card changes (Loop in…, End thread). |
| **C. Questions go where the memory is** | 13755553, 1caffb9c, 3d3d3000, ca40411c, 6191c26f, aebb3a36, 3f8f8057, 1dd637de, 153d9eb2, 0208f35d, d78c6ebb, 42b611f5, 9de5dcab, e0f63b82, b9f21ae9, 7812b61a, 2609ae5e, 5a670d95 | **Adapt.** Keep: the separate ask session goes; an ask naming a thread is answered there; a question back reaches the delegating conversation without a new card while the delegation stands; failures reach the asker. Change: an ask to a busy main conversation is answered by a seeded work thread plus an aside back (decision 1), not queued behind it. |
| **D. The delegator hears every turn** | 9af33321, 53c11c1a, 75af8085, b21b2ccd, 6758ae6a, 12e0705d, part of d915e505 and 362b5cd1 | **Replace with asides.** A handed-over thread leaves its opener a `note` per turn and a `nudge` on its result. Kelviny's wake rules (only when idle, once for many threads, counted on the pair's brake) become the `nudge` rules. |
| **E. Stop vs Interrupt** | 8ca12665, 0d12fb05, c5838795, ffca1aed, c1a237ca, ac90e42b, d10cfdfe, 34dc60d5, 3df48bcc, 9d946917, fa668c7d, 39d32095, 479fbcfa, 185120fb, 7c32e5c8 | **Take**, after C. The person's Stop ends a handoff for good and its sender hears once (an aside, not a wake); Interrupt only redirects. Adds `interrupt` to `session.cancel` and `thread.cancel`. |
| **H. Across a desk link** | e79d5890, 362b5cd1, ffca1aed, 34dc60d5 | **Adapt** to addressable threads and asides across a link (§3). |
| **Separate** | 9ea0a533 | **Review on its own.** It auto-allows ACP permission requests for Hotline's own loopback MCP tools: a security change that needs its own decision. |
| **Docs and test upkeep** | fe49127b, 16c35d54, 54ae2646, 6e5974d8, 7667a0df, 2fd22fd7, 035f7367, cc847b2a, ae62d379, 7a8ed4e7 | **Rewrite** to this design as each phase lands; the tests come along with their phase. |

## Order of work

Each phase is its own branch off `feat/thread-awareness` (itself off
`release/0.46.0`), green on `make check` before it merges into that branch.
Nothing reaches the release branch until George has looked.

1. **Foundations:** G and F, and task-316 (an "Always" grant that survives;
   the informed check reads the fold).
2. **Thread addressing and handed-over threads:** A and B.
3. **Asides:** the type, store, grants, levels and brake, delivery into the
   next turn, search; D replaced by asides.
4. **The thread index:** built per turn for the main conversation, work
   threads and calls, capped, with chapters as ended main-conversation
   threads.
5. **Asks:** C adapted to decision 1.
6. **The desk's window:** the aside line, Noted, Loop in…, End thread.
7. **Stop vs Interrupt:** E.
8. **Across a link:** H, and files between agents.
9. **Inbound shape:** the source grant model and its checks, no transport.
10. **The phone:** the aside line, Noted, Loop in… (hotline-mobile board).
