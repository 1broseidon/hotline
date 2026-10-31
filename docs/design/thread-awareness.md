# Threads as streams of thought: awareness, asides and agent to agent

Draft for George's walkthrough, 2026-10-30. Builds on
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

## What to take from #203

| #203 idea | Under this frame |
| --- | --- |
| A handoff keeps its thread, continued by name | **Take.** It's thread addressing. |
| The person talks in a handed-over thread | **Take.** |
| Asks answered by the main conversation or the named thread, ask sessions gone | **Take, reshaped.** The answer comes from a stream with memory. Open question below on waiting behind a busy main conversation. |
| The delegator hears every turn | **Replace with asides.** A handed-over thread asides its opener (`note` per turn, `nudge` on its result), instead of reports that wake it. |
| Stop vs Interrupt | **Take**, as wire and UX polish. |
| Requests end only when someone ends them | **Take.** |
| A rebuild isn't a revocation (authority vs generation); lock-order fix | **Take now.** These are bug fixes, and they overlap task-316. |

Credit: commits taken as they are keep Kelviny's authorship; adapted work
carries `Co-authored-by: Kelviny Moura`; the release notes thank him by name.

## Open questions for George

1. **Asks to a busy main conversation.** If the colleague's DM is mid-turn,
   does an ask wait for it (real memory, slower), or is it answered by a work
   thread seeded with the DM's index and recent asides (fast, nearly the same
   memory)?
2. **Aside name and placement.** Is "aside" the word, and is a Noted list in
   the teammate menu the right place to see them?
3. **Index in every thread?** Should a job's firing or a subagent's run get
   the full index, or only the DM and work threads?
4. **Inbound threads.** Worth designing now, so webhooks and email arrive as
   asides from day one, or later?
5. **Multi-party threads.** In scope, or does the aside model cover enough of
   it?
