# The agent inbox: one way to reach a teammate

A prototype on `proto/agent-inbox` (task-455), to watch how models behave
when they sort their own messages. It replaces `message_teammate` (ask and
handoff) and a colleague's `aside` with one path. It builds on
[thread-awareness.md](thread-awareness.md), whose §2 and §3 it supersedes for
teammates, and on the exchange queue of [sessions.md](../sessions.md).

## Why

Before this, the sender chose how a colleague was reached: an ask became a
turn in the colleague's main conversation, or a seeded work thread while it
was busy; a handoff opened a work thread at once; an aside was heard at the
next turn, and a nudge woke an idle colleague. So the sender decided when the
receiver was interrupted, each message was handled blind to the others, and
messages that arrived together caused conflicting work.

## The rules

These are George's, and the prototype's acceptance criteria.

1. **There is one way to reach a teammate.** An agent never chooses between
   two channels. `send` is the only tool that reaches a colleague;
   `message_teammate` and `aside` are not offered to any thread.
2. **The inbox sorts; it does not hide.** An item still wakes an idle
   receiver, a short gather delay after it lands, so a lone message behaves
   as a direct message did. But the receiver sees every unread item together
   and decides which to do first, which to combine and which to leave.
3. **A busy receiver is told, not interrupted.** It gets a short notice, "2
   new messages in your inbox: Title 1 from Ada (waiting on you), Title 2
   from Bob", and decides itself whether to read now or later, as a person
   reads mail in the middle of building a presentation.
4. **The sender can see where its item stands** without anyone writing a
   reply: sent, seen and not yet taken, taken, for later, closed.
5. **No relevance gate.** Nothing scores items or decides which ones notify:
   every item follows the same rule, so the raw behaviour can be observed.
6. **No chat opens between teammates.** Nothing is written to a pair's thread.

## The tools

- **`send`** `to` (one teammate, several, or `["team"]`), `title` (kept as
  one plain line: control characters, line breaks and characters a reader
  never sees, such as direction overrides and zero-width ones, are dropped;
  at most 80 characters, a longer one cut at a word), `message` (optional),
  `wants` (`answer`, `work` or `nothing`), `waiting` (the sender has stopped
  until this is answered), `about` (a thread of the receiver's, `kind:key`),
  `reply_to` (an item in the sender's own inbox), `files`. It returns at once
  with each item's id. `wants` and `waiting` are described to agents as facts
  about the sender, never as urgency.
- **`read_inbox`** without `ids` lists the open items, newest first: id, title,
  sender, `wants`, `waiting`, who else the send reached (`alsoTo`), state and
  how long it has been in it, and age, never the words. `list` (`open` by default) may say `closed`, the closed
  items with each one's outcome, or `sent`, the items the teammate sent
  others, each with whom it went to and where it stands now, so a sender sees
  "seen, not yet taken"; each the newest 20, with how many older ones there
  are. With `ids` (any unique prefix of four characters or more) it returns
  them whole and marks them seen.
- **`update_inbox`** `items`, a list in the order the receiver means to work
  them. Each entry names an item and sets `state`: `taken`, `later` (with a
  `reason`, kept as one plain line of at most 200 characters, as a title
  is), `closed` (with an `outcome`: `done`, `noted` or `dropped`;
  `expired` is Hotline's and refused) or `combined` (with `into`, the item it
  folds into). An entry may not answer Hotline's notice or take it into a
  thread: there is no colleague behind it. An entry may carry an
  `answer`, which replies and closes the item as `answered`, at most as long
  as a message; `answered` is refused as an `outcome`, so an item reads
  Answered only when a reply went back. `in_thread` (with
  `taken`) says whether the work runs in a work thread of the receiver's: true
  by default for an item that wants work, false otherwise. Every entry is
  checked before any moves, so one mistake changes nothing.

`list_teammates` and the rest are unchanged; `post_to_chat` also takes the
one post of a quiet turn of the main conversation (below). The lead's voice
front on a desk call has `send` in place of `message_teammate`, for named
work only, by the call's grant.

## An item and its states

An item (`InboxItem`) has an id, when it was sent, who sent it and from which
of its threads (`from`, `fromThread`), whom it is for (`to`) and whom else
the same `send` made an item for (`alsoTo`, absent when it went to one and
on an item kept before it was recorded), the title, the
words (`body`, with the whole path of each file it carried), `wants`, `waiting`, `replyTo`,
`about`, its `state`, and every state it has been in with its time
(`receipts`). Once taken into a thread it names the thread (`work`) and the
exchange request the work runs as (`request`). Once a pick-up has offered it
it carries when it is offered again (`offerAgain`), and Hotline's own notice
carries what it is about (`notice`).

```
new ──▶ noticed ──▶ seen ──▶ taken ──▶ closed (answered | done | noted | dropped | combined)
  │        │          │        ▲
  └────────┴──────────┴──▶ later ──┘
  └────────┴──────────┴──────┴──▶ closed (expired: three days open, by Hotline)
```

- `new`: it landed. `noticed`: its title was put in front of the receiver,
  at a turn's start or in a mid-turn notice. `seen`: `read_inbox` returned
  its words.
- `taken`, `later` (with the reason) and `closed` (with the outcome, and for
  `combined` the item folded into) are the receiver's sort. A `later` item can
  be taken or closed afterwards; a closed one is over. A `later` item heard in
  the main conversation, and one there the receiver was shown and left
  unsorted (`noticed` or `seen`), is offered to it again, quietly, the first
  time the conversation is idle after a turn, then by its own clock alone, an
  hour later and then daily (the pick-up, below).
- `expired` is Hotline's outcome, never a sort: an item still open (`new`,
  `noticed`, `seen` or `later`) three days after it arrived closes by itself
  (the clock, below). A `taken` or held item never expires.
- An item that the person's first-contact card has yet to allow is kept
  `held`: the receiver cannot read it, and it does not count as unread. A yes
  lets it in as `new`; a no, an expiry or the person's Stop exchange closes it
  `dropped`, with why, unseen.

Items are kept on the receiver's own stream, `inbox/<personaId>.jsonl`
(`StreamId::Inbox`), one line per item, written again under its id at each
change: the fold is the whole state, and a restart reads it back as it was.
At start-up a held item waits on its card again, an item no turn has been
offered wakes its receiver, and what a teammate set aside is offered to it
once (the pick-up at start, below), so a `later` item does not sit until the
person next writes.

## When an item reaches the receiver

An item is heard in one thread of the receiver's: the thread it is `about`
while that thread is live (a reply to something sent from a work thread is
about that thread), else the receiver's main conversation.

- **Idle.** An item that lands while that thread is idle wakes it once, after
  the gather delay, `HOTLINE_INBOX_GATHER_MS` (default 4000 ms; a unit test's
  room gathers in 150 ms). Items that land within the delay are heard in one
  turn. A teammate that is not running is started for it, as one was for a
  colleague's question. The wake line carries titles, senders and waiting
  marks, never bodies.
- **The quiet turn.** The turn of the main conversation that the inbox wakes
  is quiet, as a quiet job's firing is ("I think like scheduled tasks it can
  run quiet and escalate to the user when needed"). Its line begins `quiet
  inbox` and says who reads it: nobody; the person hears only what it posts
  with `post_to_chat`, once that turn, which it should when an item's outcome
  is theirs (a result they asked it to get, a decision that is theirs, a
  problem) and not for a colleague's question it simply answered. Its words
  are written as thinking, the path under the turn: no bubble, no unread on
  the rail, no notification or push, nothing a call says. Its post is an
  ordinary message from the teammate. It runs in the main conversation's own
  session and context, never a thread or a clone of its own. The person's
  line steered into it ends the quiet, and the turn answers them; a work
  thread or a call the inbox wakes is not quiet.
- **Busy.** An item that lands mid-turn starts no turn and no work thread.
  When the gather delay is up, the turn in flight in the main conversation is
  handed the one-line notice, steered into it as the person's mid-turn line
  is, at most once per gather window: `HOTLINE_INBOX_MIDTURN=0` (or `off`,
  `false`) turns this off. A notice never makes a turn loud: steered into a
  quiet turn it leaves it quiet, being no line of the person's, and one a
  driver that takes no line mid-turn leaves waiting starts, as its next
  line, the same quiet turn the idle wake starts, with its line. A busy work
  thread hears its items at its next turn's start; it is not handed a notice.
- **At a turn's start.** Every turn of the main conversation, a work thread
  or a call opens with an `INBOX` section in its thread index: the count of
  unread items and items for later, the new items not yet offered to a turn
  (title, sender, `waiting on you`, what it wants, age, short id, and which
  item it answers), and what moved among the items the teammate sent. An item
  is offered once; the count stays while it is unread. The section is fenced
  as data (`hotline_inbox`), and every title and reason in it is quoted with
  its own quotes escaped (`fence::quoted`), so none can end its quote, forge
  a second entry or speak as Hotline. The mid-turn notice, the pick-up's
  line, the expired notice's words and the thread index quote them the same
  way, and a work thread taken from an item hears its title only as the
  first line inside the colleague's own fence.
- **At a turn's end.** Items still unread that no turn's start was offered,
  because they landed mid-turn, wake the idle thread as above. An item wakes
  its receiver once as new; what the main conversation was shown and left
  open waits for the pick-up.
- **The pick-up** (`inbox/pick_up.rs`). When a turn of the main conversation
  ends and the conversation is idle, and items heard there are open that the
  teammate has not finished sorting (`later`, or offered or read and left
  unsorted; not `taken`, not `closed`) and that no pick-up has offered yet,
  it gets one quiet inbox turn, by the
  same mechanism and with the same quietness and `post_to_chat` gate as the
  quiet turn above. It comes the gather delay after the turn ended, and only
  if the conversation is still idle with nothing queued and no turn has begun
  or ended since: anything that starts a turn first (the person's line, a new
  item's own wake, a post) wins, has the items in its index, and arms its own
  pick-up when it ends. Items no turn has been offered yet are left to their
  own wake. Its line begins `quiet inbox`, says the teammate is idle now and
  these are items it set aside, and lists them as the index does (title,
  sender, `waiting on you`, what it wants, age, short id) with where each
  stands and each `later` one's reason, fenced as data.
  - **No loops.** A turn the pick-up started arms no pick-up when it ends,
    whatever it leaves open: the next can only follow a turn something else
    started. A teammate that answers `later` every time settles after one.
  - **Once an item.** A turn's end offers an item once. The person's turns
    after that offer it nothing, however many there are: it has its own
    times (below).
  - A turn the person stopped or interrupted, or one cancelled or failed, arms
    none, and the person's Stop while one waits disarms it. A teammate with
    nothing open, one not running or removed, and a room that is closing get
    nothing.
  - **When the desk opens**, once the room has handed on what it resumes
    (kept lines, job deliveries, deliveries never read, never-offered items'
    wakes; `recover_exchanges`, a few seconds after start), each teammate
    whose main conversation has items set aside is offered them once in the
    same quiet pick-up turn, started for it if it is not running, as the
    new-item wake starts one. Teammates are offered a gather delay apart,
    one after another, so a launch never starts them all at once. It is
    offered only while the conversation is idle with nothing queued and has
    had no turn since the desk opened (a turn it had offers by its own end,
    and a never-offered item's wake wins), once per launch, and its turn arms
    nothing: a teammate that defers again waits for its items' own times.
  - **Again, on the items' own time** (`inbox/clock.rs`). Each item a
    pick-up offers carries when it is offered next, `offerAgain`: an hour
    after its first pick-up, then a day after each. When that time has come
    and the main conversation is idle with nothing queued, one quiet pick-up
    turn offers every item that is due (one not yet due waits). A turn's end
    offers an item once, the first time the conversation is idle after it
    was set aside; after that only its own time offers it, however many
    turns the person starts, so a busy conversation is offered a due item at
    the clock's first look once it is idle again. The times are the items',
    not a turn's, so the no-loop rule stands: a pick-up's turn arms
    nothing. A teammate not running is not started for a
    timed offer; a desk that was closed when one came due makes it at start,
    by the pick-up at start, which counts as the offer.
  - Work threads and calls have no pick-up.
- **Expiry** (`inbox/clock.rs`). An item still open three days after it
  arrived (when it was let in, for one the first-contact card held) closes
  as `closed`, outcome `expired`, with its receipt and the reason "Not picked
  up in 3 days.", so its sender's row and `read_inbox list: sent` and the
  receiver's closed list and the desk's panes say Expired, and the sender is
  told at its next turn as of any move. The sender also gets one item back,
  Hotline's notice: a reply to the expired item (`replyTo`), `wants:
  nothing`, `notice: expired`, from the receiver it is about but shown to
  teammates as from Hotline, titled `Expired: <title>`, whose words are the
  desk's ("This is Hotline's notice, not Bob's words…") and give the
  receiver's last reason for later. It lands as any item does, so an idle
  sender is woken quietly. When the expired item came from something the
  person asked for (`forPerson`: it was sent in a turn of the sender's main
  conversation that the person read, not a quiet turn, a thread or a job),
  the notice tells the sender to say so to the person in one line with
  `post_to_chat`: that the colleague never picked it up. Otherwise the
  sender decides whether the person needs to know. The rules are the same
  either way: three days, unread items too, and a teammate not running is
  still not started only to be offered a set-aside item again. The receiver is not woken. A notice can expire too, and an expired notice sends
  none. Notices count on no brake: they are no teammate's message, and they
  are not among what the receiver sent.
- **The clock** looks when the desk opens (after the room has handed on what
  it resumes: what expired while it was closed closes first, then the pick-up
  at start) and then every 30 seconds.
- **Switches.** `HOTLINE_INBOX_GATHER_MS` (4000), `HOTLINE_INBOX_MIDTURN`
  (on), `HOTLINE_INBOX_OFFER_AGAIN_MS` (3600000, an hour after the first
  pick-up), `HOTLINE_INBOX_OFFER_EVERY_MS` (86400000, a day after each later
  offer), `HOTLINE_INBOX_EXPIRE_MS` (259200000, three days since it arrived).
  A unit test's room uses 150 ms, a minute, two minutes and five minutes, and
  reads the clock at a time it chooses.

## Taking work, answers and results

Taking an item into a thread (`taken`, `in_thread` true) makes an exchange
request from the item's sender to the receiver, `intent: handoff`, carrying
the item and the items combined into it (`exchanges/inbox.rs`): their titles,
senders and words seed one work thread, whose first line is the taken item's
title. The request is the one a handoff always was, so the same machinery
runs it: a place among the receiver's live threads (`MAX_LIVE`, three,
waiting its turn when every one is mid-turn), cards in the thread, the
person's Stop and Interrupt, restarts. The receiver starts it; the sender's
`send` never does. It needs no card of its own: the item's grant was checked
when it was sent. An item is taken into one work thread only: it is claimed
for its request under the inbox lock before the work starts, so of two takes
at once one opens a thread, and any later take of it, in a thread or in the
turn, is refused with the thread that has it (its title and key once it is
open). The record of that thread is never overwritten.

A work thread taken from an item may `post_to_chat` with `wake`: the main
conversation is handed the post fenced as data (`hotline_posted`) under
where it came from, told that the thread is work taken from the colleague's
item, so what it asks for is the colleague's, and that it is a report, not
an order (`post.rs` `Room::woken_by`).

An `answer` on `update_inbox`, and the result of a work thread a taken item
started, are new items in the original sender's inbox with `replyTo` set,
`wants: nothing`. The original closes as `answered` for an answer and as
`done` for a work thread's result (as `dropped`, with the failure, when the
work failed). A result goes to each sender whose item the
work carried, once. If the original was sent from a work thread that is
still live, the reply is heard there; otherwise in the sender's main
conversation. A reply wakes an idle sender by the same rule.

The person's Stop of taken work, or End thread on its thread, ends the
request without a result and closes its items as `dropped`, with the cut
turn's words and the person's sentence; nothing wakes. Loop in… puts an item
from the thread's teammate in the colleague's inbox, marked `loopedIn`, its
answer heard back in the thread; the thread keeps a looped-in `aside` line
for the window to draw.

## What the sender is told

Every change of an item is a receipt with its time. At the start of its next
turn in the thread the item was sent from (if live, else its main
conversation) the sender is told, under `SENT · what moved`, each of its items
that moved since it was last told, as one line of where it stands now:
`"Pick a name" to Bob · later 14:03: "after the release" · 1a2b3c4d`. Only the
changes, like the index. The window reads the receipts over the wire.

## Guards

- **The person's word.** Only the person's own lines carry the person's
  authority. A colleague's "the person asked for this" is the colleague's
  claim, and the receiver is told so where it reads an item: `read_inbox`'s
  note, a taken item's delivery and its work thread's brief, the tool text
  and the room skill (`inbox::PERSONS_WORD`).
- **First contact.** `send` checks `authorize_collaboration` as an ask or an
  aside was checked, and for an item that wants work as a handoff was
  (informed consent). An item the card has yet to allow is held, as above.
  The card is asked of what the sender says it wants: a receiver may take an
  item that wanted an answer into work, and the card that let it in stands
  for that work.
- **Files.** A file an item carries lands outside the receiver's working
  directory, in a folder Hotline keeps for it on the desk,
  `received/<receiver>/<sender>/`, which its own sessions are granted as
  one more read-only extra folder (task-467, decision-47). The receiver
  reads it there and copies it into its project itself, with its own reach,
  so no colleague puts a file a tool picks up by its name (a test runner's
  config, an instructions file) in a teammate's tree
  (`docs/sessions.md`, Files between teammates).
- **Whole-machine reach.** These guards bind what a teammate asks through
  Hotline's tools. A whole-machine teammate, or a harness with its own
  shell, can change the desk's records on disk, and is trusted as the
  person is (`docs/security.md`, Accepted risks).
- **The brake.** Every item and every reply counts on the two teammates'
  `EXCHANGE_CAP` (12); at the cap they pause, the person has the card, and
  nothing more is sent or answered between them until the person answers
  it: a line from the person about anything else does not. Keep going counts
  afresh. Stop exchange ends their outstanding work and holds them apart:
  `send`, an `answer` and taking work into a thread between the two are
  refused with a plain reason until the person next writes to one of them,
  which starts the count afresh. Their open items stay open for the receiver
  to close or leave. The result of a work thread is counted as a request's
  reply always was.
- **One path.** Nothing reaches a teammate by `ask_in_main`,
  `ask_in_own_thread`, a sender-triggered `dispatch_handoff`, or
  `aside_from`/nudge: those are deleted. An ask from before the inbox that is
  still open when this version starts is ended as unanswered and its sender
  told, in its inbox, to send it again.

## What was removed

`message_teammate` and `aside` (tools, handlers, glance labels and guidance);
`Room::send_intent` and the request queueing behind it; asks routed to a main
conversation, to a work thread of their own, back to a delegator, or into a
named thread (`exchanges/questions.rs`), and the work thread's brief for a
question; asks as gates on a handoff (`gate_on_ask`); continuing a handoff in
a named thread (`continues`, `handoffThreads` in `list_teammates`) and the
follow-up that steered it; the per-turn note and result nudge a handed-over
thread left its opener, and the note of the person's Stop or End thread (the
item's receipts say it now); lines on the pair's thread; a colleague's aside
and its crossing to a linked desk; the linked desk's requests, notes and asides
arriving here, and files crossing a link.

## Out of scope

- **Linked desks.** `send` refuses a teammate on a linked desk with a
  sentence, and a linked desk's request or aside is refused at
  `federation.deliver`. The link still carries rosters, and the answer to a
  request an older version forwarded.
- **Inbound sources** stay on their aside path (`inbound.rs`, `inbound_aside`),
  with their own grant, level and brake; turning them into inbox items would
  change the source grant model.
- **A work thread waiting on a reply.** A taken item's thread that `send`s
  with `waiting: true` still ends its turn and returns its result; the reply
  is heard in that thread only while it is live. Asks used to hold a
  handoff's result back as a gate.
- **Mid-turn notices in work threads**, jobs and runs; and inbox items heard
  in a job's thread (a reply to something a job sent is heard in the main
  conversation).
- **Relevance**: no score or gate decides which items notify.

## On the desk

The window draws the inbox from the contract below (task-456). The person
reads; the teammates act: nothing on the desk sends, sorts or answers an
item, and no chat opens between teammates.

- **Sent rows.** Each `inbox_sent` on a thread is a quiet row like a thread's
  link: the tray, the title in bold, whom it went to and where it stands now
  ("To Juno · Taken"). The items one `send` made to several teammates under
  one title stand side by side on the tape and are one row ("To Juno, Ivy,
  Huck · 2 of 3 seen", or the word they all share). The state is read live
  from the sender's `{"inbox"}` subscription, not from the line. A press
  opens the item in the right-hand pane (`ui/src/inbox.ts`,
  `components/transcript/InboxLines.tsx`).
- **The sort's line.** Each `inbox_sorted` on the receiver's main
  conversation is one quiet line where it fell, "Read 3 messages" ("Read 1
  message"), drawn as a line of a turn's path is (the dot, the words, the
  chevron; `FoldLine` in `components/transcript/Trail.tsx`, which the
  path's "Started 4 helpers" line also is). Pressed, it unfolds to a plain
  row per item in the order given: its title, its sender and a later's
  reason in the muted ink, and what became of it at the right (Taken,
  Later, Answered, Done, Noted, Dropped, Combined, Expired: the item's
  outcome, in the word the sent row and the panes use). No card, header, counts or
  numbering. A press on a row opens the item. Work a sort took is its own
  work-thread row (below).
- **A quiet turn.** The turn the inbox woke draws as path, not as a reply:
  its sort's line and its sent rows where they fell, its tool rows and its
  words, as thinking, in the turn's collapsed steps under them, the line any
  turn's steps have, and no bubble unless it posted. A sort in the middle of
  a turn the person started is the same one line where it fell, above the
  reply; it does not split the turn's steps, which stay one run under the
  reply. A turn that ends without a new line in the chat posts no desktop
  toast (`ui/src/notify.ts`).
- **The item pane.** Its title, with where it stands in a word beside it
  (Sent, Seen, Taken, Later, Answered, Done, Expired…, the sent row's word);
  from whom to whom, whom else the send
  reached ("also sent to Quill"), what it wants, and whether the sender
  waits; the words; Receipts, a timeline of sent,
  seen, taken, later with its reason, closed with its outcome
  (Expired, with "Not picked up in 3 days.", for one that expired; and
  "Allowed" when the first-contact card let a held item in); the work thread
  taking it started, which opens that thread's pane (beside the receiver's
  conversation when the open teammate's Threads list does not hold it); the
  item it answers and the items that answer it. A held item says "Waiting
  for you to allow it." A receipt from before today gives its date as well
  as its time. Hotline's notice is from Hotline, by its mark,
  wherever an item's sender is named (the pane, its rows, the sort's row,
  the composer's tray). Back goes where it was opened from.
- **One word for each outcome.** An item a reply went back on is Answered
  and one whose work finished is Done, the same word on its sent row, its
  sort's row, the inbox pane and the item pane. Noticed, the desk showing
  the receiver a title, is the desk's own step: an item in it reads as Sent,
  and the receipts leave it out. In the Closed list a finished item is
  ticked, a dropped one crossed, and an expired one, never picked up, keeps
  a dotted ring.
- **The inbox pane.** Open (Unread: new, noticed and seen; Taken; For
  later), Closed and Sent, newest first, under a line of the open counts. An
  unsorted item whose sender waits counts how long in amber. It opens from
  the teammate's menu (Inbox, beside Threads, with "N unread") and from the
  composer's tray.
- **Work an item started.** The desk opens a taken item's work thread with
  the item's sender as its opener, as a handoff was. The window tells the two
  apart by the inbox views (`item.work`): such a thread is a work thread
  everywhere. On the receiver's conversation it is a link row with the live
  dot, its title and "Work thread · Running" (or Open, Parked, or what came
  of it), and on the sender's "Work thread · Juno · Running"; the Threads
  list and the composer's count list it as a work thread; its pane is the
  work thread's, with the item's words over it as its Message. A handoff from
  before the inbox keeps its old line and pane.
- **The composer and the rail.** While the open teammate has unread items,
  the composer's bottom row has "N unread" with the tray, beside "N threads"
  and in its style; it opens a tray of them (title, sender with "waiting"
  when the sender waits, and when each arrived, the same on every row) with
  a last row to the whole inbox. The rail shows no inbox count: a
  teammate's unread is its own business, and the chip says it the moment
  the person opens that conversation.
- **What was removed.** The teammate pane's Threads section, which listed a
  teammate's conversations with colleagues. Old pair threads and colleague
  asides still on disk keep rendering where they always did (the Threads
  list's Pairs and From teammates, `PairPane`, `AsideLine`), and inbound
  sources' and Loop in's asides still draw as asides.

## Contract

Types (`crates/hotline-contract/src/inbox.rs`, generated into
`ui/src/generated/contract.ts`):

- `InboxItem` as above; `InboxReceipt {state, at, reason?, outcome?, into?}`;
  `InboxState` `new | noticed | seen | taken | later | closed`; `InboxOutcome`
  `answered | done | noted | dropped | combined | expired`; `InboxNotice` `expired`;
  `InboxWants` `answer | work | nothing`. `InboxItem.notice?` marks Hotline's
  own notice, `InboxItem.offerAgain?` is when a pick-up offers it next, and
  `InboxItem.forPerson?` marks one sent in a turn the person read.
- `InboxView {personaId, open, closed, sent, unread}`: open and closed newest
  first, what the teammate sent others newest first, and its unread count.
- `InboxItemDetail {item, replies}`: the item with its receipts and its `work`
  thread, and the items that answer it.
- `RosterEntry.inbox?`: how many open items the teammate has not read, which
  the teammate's menu says beside Inbox.

Wire commands ([wire.md](../wire.md)):

| Command | Params | Result |
| --- | --- | --- |
| `inbox.list` | `{personaId}` | `InboxView` |
| `inbox.item` | `{id}` | `InboxItemDetail` |

Subscription: `{"inbox": "<personaId>"}` sends the `InboxView` as its
snapshot and again, whole, whenever an item the teammate holds or sent
changes (every change wakes both teammates' roster rows, which the view
follows); `removed` when the teammate is deleted. The roster row's `inbox`
count moves on the same push.

Transcript events (`TranscriptEvent`):

- `inbox_sent {id, ts, item, to, title}` on the sender's thread, once per item
  sent (a reply included); the item's live state is the receiver's, read with
  `inbox.item` or the subscription.
- `inbox_sorted {id, ts, entries: InboxSortedEntry[], open, unread}` on the
  receiver's main conversation each time `update_inbox` runs, with
  `InboxSortedEntry {item, title, from, state, reason?, outcome?, into?,
  notice?}` in the order given (`notice` on Hotline's own, whose sender is
  Hotline), and how many items are open and unread
  after it.
