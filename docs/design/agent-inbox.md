# The agent inbox: one way to reach a teammate

How teammates reach each other from 0.50 (task-455): one path, an item in
the colleague's inbox, which the colleague reads and sorts itself. It
replaces 0.49's `message_teammate` (ask and handoff) and a colleague's
`aside`. It builds on [thread-awareness.md](thread-awareness.md), whose §2
and §3 it supersedes for teammates, and on the exchange queue of
[sessions.md](../sessions.md). What a desk updated from 0.49 had in flight
carries on as below (One path); a teammate on a linked desk is not reached
yet (Not in 0.50).

## Why

Before this, the sender chose how a colleague was reached: an ask became a
turn in the colleague's main conversation, or a seeded work thread while it
was busy; a handoff opened a work thread at once; an aside was heard at the
next turn, and a nudge woke an idle colleague. So the sender decided when the
receiver was interrupted, each message was handled blind to the others, and
messages that arrived together caused conflicting work.

## The rules

These are George's, and what the inbox holds to.

1. **There is one way to reach a teammate.** An agent never chooses between
   two channels. `send` is the only tool that reaches a colleague;
   `message_teammate` and `aside` are not offered to any thread, and a
   call to either is told, in one line, to use `send`.
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
   every item follows the same rule.
6. **No chat opens between teammates.** Nothing is written to a pair's thread.

## The tools

- **`send`** `to` (one teammate, several, or `["team"]`), `title` (optional;
  kept as one plain line: control characters, line breaks and characters a
  reader never sees, such as direction overrides and zero-width ones, are
  dropped; at most 80 characters, a longer one cut at a word), `message`,
  `wants` (`answer`, `work` or `nothing`), `waiting` (the sender has stopped
  until this is answered), `about` (a thread of the receiver's, `kind:key`),
  `reply_to` (an item in the sender's own inbox that this answers, or one it
  sent that this follows up on), `files`. Its description tells a teammate
  the two ways to write: a quick question or answer is one short line in
  `message` with no `title`, which the person reads like a chat; a hand-off
  is a `title` and a full brief, which the person reads like a letter. A
  quick question is answered with a quick line on `reply_to`. Without a
  `title`, the message's first plain line is the title (a leading `#`, `>`,
  `-` or `*` dropped, cut at a word to 80 characters), and the whole message
  stays the words, unless that line is the whole message, which is then
  kept as the title alone so it is never said twice. A send needs a
  `message` or a `title`. An answer (`reply_to` naming an open question in
  the sender's own inbox, sent back to whoever asked it) closes that
  question as Answered, as `update_inbox`'s `answer` does, and wants
  nothing back unless it says so. A text-only answer to the asker alone
  needs no new first-contact card, and goes back to the thread or job run
  that asked while it is live, as an `update_inbox` answer does. Extra
  recipients, files, and requests for an answer or work use the ordinary
  send grant. A reply to an item that wants work is a question about it
  and leaves it open. It returns with each item's id and
  where each landed, per recipient and in the desk's words, first in its
  note: "Huck: in inbox · Quill: not sent yet, waiting on the person's
  card"; a held item is never said to have arrived (`delivery` on the
  result and on each `sent` entry). A send that needs no card returns at
  once. One that raised a first-contact card (Up in access, below) waits
  for the person's answer, up to 45 seconds (`HELD_SEND_WAIT`), so the
  teammate's one message about it is true:
  - **Yes in time:** "in inbox", with the same note as a send that needed
    no card.
  - **No:** "not sent, the person declined"; the note says nothing was
    sent, the item was closed as dropped, unseen, and not to send it again
    or reach that colleague another way for it. A card taken down
    unanswered (an expiry, the person's Stop) says "not sent" and why.
  - **No answer by then:** "not sent yet, waiting on the person's card";
    the note says not to say it was sent or delivered, to say one short
    line that a card needs their answer before the teammate can do what it
    was for and nothing else about it, that it goes by itself once allowed
    and the teammate is told at its next turn, not to send it again, and
    that `read_inbox` with list `sent` says where it stands. On a call (a
    call thread, or a main-conversation turn said on a call to the
    teammate) it says instead that the call announces the card itself, so
    the teammate says at most that it is waiting.

  A send to several waits for each card it raised against the one 45
  seconds and says each recipient's line. The 45 is under the shortest
  bound an agent puts on a tool call: Codex gives an MCP server's tool 60
  seconds unless its config says otherwise (`tool_timeout_sec`, which
  Hotline does not set); Claude Code allows hours, and Hotline Agent calls
  in process with no bound. The wait holds no lock the card's answer or
  anyone's inbox needs, and ends with the turn: Hotline Agent drops the
  call when its turn stops, and an ACP agent's cancelled call is dropped
  when its MCP cancellation arrives. Neither touches the card or the item,
  which go on as below. `wants` and `waiting` are described to agents as facts
  about the sender, never as urgency. Sent in a turn the person reads, its
  result adds that the person sees the message as a row with where it
  stands, so the teammate does not say it sent it or will pass on its
  answer, and says only what the row does not, or nothing (task-505); the
  house style's opening line for something to pass to a colleague is the
  send itself, with no words, for the same reason. A quiet turn's send is
  told no such thing.
- **`read_inbox`** without `ids` lists the open items, the newest 50 with
  how many older ones there are, newest first: id, title,
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
work only, by the call's grant. `FrontWork` describes and checks that
narrow `send`: named recipients, a title, the person's words, and `wants`
fixed to `work`. It refuses broadcast aliases and fields the front is not
shown. For a question, a note or a check the front uses `work_on_it`, whose
working session on the call thread holds the general `send`.

`Room::deliver` keeps each colleague item once, for `send`, `update_inbox`
answers, work results and Loop in. A held send is kept unseen until its card
settles that same item; it counts and wakes the recipient only upon landing.
An exchange has already counted its work result. Loop in keeps its own quiet
transcript line instead of a sent row. Hotline writes its own notices
separately; they are not colleague deliveries.

## An item and its states

An item (`InboxItem`) has an id, when it was sent, who sent it and from which
of its threads (`from`, `fromThread`), whom it is for (`to`) and whom else
the same `send` made an item for (`alsoTo`, absent when it went to one and
on an item kept before it was recorded), the topic it is in (`topic`) and
the send that made it (`sendId`, below), the title, the
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
  `dropped`, with why, unseen. A `send` whose result already said which
  (it waited, above) marks it told, so the next turn does not say it again;
  one answered after the wait is told at the sender's next turn.

Items are kept on the receiver's own stream, `inbox/<personaId>.jsonl`
(`StreamId::Inbox`), one line per item, written again under its id at each
change: the fold is the whole state, and a restart reads it back as it was.
At start-up a held item waits on its card again, an item no turn has been
offered wakes its receiver, and what a teammate set aside is offered to it
once (the pick-up at start, below), so a `later` item does not sit until the
person next writes.

An item a newer Hotline wrote is left as it found it, as a pair record is
([log.md](../log.md#the-room-stream)): a field this build does not know, on
the item or a receipt, is written back as it was at every move here, and an
item in a state or with an outcome this build does not know is never moved,
expired, woken or swept. `read_inbox` lists it with its state and outcome as
written, reading it shows its words without marking it seen, and a sort or
a reply that names it is refused, saying a newer Hotline saved it. A
field a newer Hotline wrote inside a thread an item names (`fromThread`,
`about`, `work`) is written back as it was found too, while the item still
names that thread (`hotline_store::inbox::line`). The window's pane lists
such an item where it belongs (Open or Closed, by the state as written,
whether the teammate holds it or sent it) as a plain row that opens
nothing: its title, whom it is from or to, and "Saved by a newer version of
Hotline. It can't be changed here."; on Open it stands under its own
heading, From a newer version, with its count.

## When an item reaches the receiver

An item is heard in one thread of the receiver's: the thread it is `about`
while that thread is live (a reply to something sent from a work thread is
about that thread), else the receiver's main conversation. A reply to
something a scheduled job sent is about the job, and heard in it while the
run that sent it is still going: the job lives and no firing of it has
begun since (`Room::job_run_open`). So "ask Huck, then summarise" gets
Huck's answer in the job, even after the firing's own turn has ended: the
job's thread is woken for it, as an idle work thread is, with a line that
says replies came back to the run and that only `post_to_chat` reaches the
person. Once the job's next run begins, or the job ends, the run is over: a
reply to it lands in the main conversation, quietly, and one that came back
to it and was never heard is heard there, woken for it.

A job's last run, a one-time job's only one or a loop's last, waits for its
replies (`job_thread/waiting.rs`). When its turn ends while an item it sent
that wants an answer or work is still open, the job does not end: its agent
is let go of, so it runs no turn and is not working, its thread says once
"Waiting for Huck to reply. The job ends once the reply has come back.", and
its record names whom it waits on (`awaiting`), which its row in Schedules
says as "Waiting on Huck" in place of its next run, and its pane in its state
line, its Next run fact saying None; the conversation's Schedules key counts
the soonest of the other jobs as next, or says "Waiting on Huck" when every
job is waiting. Each reply is
heard in the job, a turn told it is the job's last run, and the turn that
hears the last one ends the job, its thread closed. A run that sent several
items waits for each. It never waits for ever: an open item expires after
three days as any does, and Hotline's notice of it is heard in the job as a
reply is; an item the colleague closed with no reply is heard where it
stands at the clock's next look, a turn of the job, which then ends; one
taken and never answered, which never expires, is waited on until three
days after it arrived, and the job then ends saying "No reply came in three
days, so the job has ended." The person's End job ends it at once, and so
does Stop, which says "Stopped. That was the job's last run, so it has
ended."; a reply after that is the main conversation's. A desk that
restarts keeps it waiting: the record says it is in its last run, the items
are on disk, and what came back meanwhile wakes it as the desk opens.

- **Idle.** An item that lands while that thread is idle wakes it once, after
  the gather delay, normally 4 seconds. A lone new quick message to an idle
  receiver waits 1 second; if another item lands or work begins within that
  second, the normal deadline still holds. Items that land within the delay
  are heard in one turn. A newly landed colleague item for the main
  conversation can start a stopped receiver's driver and open its saved
  session during the gather window, without a prompt or model turn. A held
  first contact starts nothing until it lands; recovery, reminders and desk
  notices still wait for the ordinary wake. Stop or a policy change withdraws
  the old gather's authority, and a failed early start is not retried by the
  same gather. The index usually carries titles, senders and
  waiting marks. If the wake offers exactly one new quick item, it shows the
  whole message as quoted sender words. The offer is not a read: the inbox
  marks the item `seen` only after the driver reports a completed model turn
  or observable model work, or the receiver explicitly reads it. This is either a
  compact title-only message or a body of at most 280 characters that asks
  for an answer or is a reply. Work handoffs, Loop in items and desk notices
  stay title-only. One `send` to the sender with `reply_to` answers a question:
  neither `read_inbox` nor `update_inbox` is needed. An answer or result that
  wants nothing back does not need a reply. This applies in the main
  conversation, work threads, calls and jobs; it does not change busy
  notices, multiple new items or anything already offered.
- **A wake that could not run.** An unread item stays open. A failed wake
  does not start the idle pick-up; the inbox clock offers it again after an
  hour, then daily, or at a known provider reset. An explicit read or answer
  can still pick it up sooner. Hotline sends the original sender one
  `wakeFailed` notice per item and failure state, in the asking thread while
  it is live; the notice neither answers the item nor counts on the brake.
  For an item sent for the person, a separate short notice appears in chat.
  The wire's `wakeFailure.state` is one of:
  - `outOfQuota`: the driver reported a quota or rate limit, including a
    refusal with the harness's known quota announcement, whether streamed
    as speech or thought. The announcement alone is not model work.
    `resetAt` is an estimated epoch millisecond time when the driver gives
    a retry interval. Chat shows the time on the desk's clock; the inbox
    notice includes its date and offset.
  - `notSignedIn`: the driver identified a provider or harness sign-in problem.
  - `stopped`: the person explicitly stopped the turn before it did model work.
  - `couldNotStart`: no session could open for the wake.
  - `other`: no more specific diagnosis. A bounded, redacted diagnostic line
    may be included as data; a thought or a silent stream alone is not proof
    of model work, an account failure, or a Stop.
- **The quiet turn.** The turn of the main conversation that the inbox wakes
  is quiet, as a quiet job's firing is ("I think like scheduled tasks it can
  run quiet and escalate to the user when needed"). Its line begins `quiet
  inbox` and says who reads it: nobody; the person hears only what it posts
  with `post_to_chat`, once that turn, which it should for a decision that
  is theirs or a problem, and not for a colleague's question it simply
  answered, an item it sorted, or progress on something the person asked
  for: that is told when it is finished (What the person asked for, below),
  and the line says so. Its words
  are written as thinking, the path under the turn: no bubble, no unread on
  the rail, no notification or push, nothing a call says. Its post is an
  ordinary message from the teammate. It runs in the main conversation's own
  session and context, never a thread or a clone of its own. The person's
  line steered into it ends the quiet, and the turn answers them; a work
  thread or a call the inbox wakes is not quiet.
- **What the person asked for** (`inbox/asked.rs`). A send made in a turn of
  the sender's main conversation that the person read, a turn started by
  their follow-up line (`aboutItem`) included, is theirs (`forPerson`). While
  colleagues work on it nothing of it reaches the person: a receipt, an
  answer from one of several, each wakes the sender's quiet turn as any item
  does. It is finished once every item the send made is closed (answered,
  done, noted, dropped, combined or expired; one held for the person's
  first-contact card is still open), however it closed: a decline lands
  nothing in the sender's inbox, so the close itself gathers the sender's
  main conversation. Its next wake there is then a turn the person reads,
  not quiet, whose line begins `inbox · Finished:`, names the send's title
  and how each colleague's part ended ("Score?" (Timo: answered)), and says
  to tell the person the result in its own words, addressed to them, in a
  line or two, and not to paste or quote the colleague's reply, which was
  written to the teammate. What it says is the reply: a bubble, a push to
  the phone. The desk makes the turn loud; the words are the teammate's. A
  turn the person reads that is in flight is told the same in a notice
  ("Tell the person in this turn"), and a quiet one in flight is followed by
  the turn they read. Finished is news once: while one of the send's items
  has closed since its sender was last told (`senderTold`). A quiet turn's
  index leaves it untold for the turn the person reads; any other turn's
  index marks its SENT lines `finishes what the person asked of you: tell
  them the result, in your own words`. A send that wants nothing back
  finishes nothing the person waits on, and a send not for the person stays
  quiet to the end.
- **Results for a call** (`inbox/on_call.rs`). A request for an answer or work
  sent on a call remembers that particular call. An admitted answer or work
  result gets one best-effort offer to the call's work agent, as steering
  if it is busy or a new turn if it is idle. The desk does not inspect the
  agent's speech to decide whether it worked and does not retry refused or
  unused steering. On hang-up (or on arrival if the call is already over),
  Hotline writes the attributed result to the asker's main chat, without
  waking that agent. The chat event is keyed by the result item, so hang-up,
  delivery and recovery cannot post it twice. Startup recovers written
  copies from the inbox; it does not recreate voice offers. Ordinary
  progress, receipts, and consent cards keep their existing routes.
- **Busy.** An item that lands mid-turn starts no turn and no work thread.
  When the gather delay is up, the turn in flight in the main conversation is
  handed the one-line notice, steered into it as the person's mid-turn line
  is, at most once per gather window. A notice never makes a turn loud: steered into a
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
  the notice tells the sender that it tells them, in one line, in the turn
  they read once everything it sent for it is finished: that the colleague
  never picked it up. Otherwise the
  sender decides whether the person needs to know. The rules are the same
  either way: three days, unread items too, and a teammate not running is
  still not started only to be offered a set-aside item again. The receiver is not woken. A notice can expire too, and an expired notice sends
  none. Notices count on no brake: they are no teammate's message, and they
  are not among what the receiver sent.
- **The clock** looks when the desk opens (after the room has handed on what
  it resumes: what expired while it was closed closes first, then the pick-up
  at start) and then every 30 seconds.
- **The times** are fixed: the gather delay normally 4 seconds (1 for a lone quick message to an idle receiver), an offer again an
  hour after the first pick-up and a day after each later one, expiry three
  days after an item arrived. Nothing on a desk changes them; a unit test's
  room uses 150 ms, a minute, two minutes and five minutes (`cfg!(test)`),
  and reads the clock at a time it chooses (`Room::inbox_due`).

## What is kept

(`inbox/tidy.rs`.) An item is a line on its receiver's stream written again
at every move, so the stream grows with every move and every item. What it
keeps, and what goes:

- **Closed items are kept 30 days**, then removed: from when they closed
  (their closing receipt), however they closed (answered, done, noted,
  dropped, combined, expired). The number is the person's, in Settings,
  General, Conversations: "Keep closed messages for [30] days" (the room
  setting `inboxKeepDays`, a whole number of days from 1 to 3650). The clock
  removes them as it looks, and the desk as it opens. Open, taken and held
  items are never removed, however old.
- **The desk tidies the inbox as it opens** (`Room::settle`): each
  teammate's stream is written again with each item's latest line alone
  (`Log::tidy`), less the closed items past their days; the work an item
  was taken into leaves the two teammates' record once it is over and its
  items are gone, and the leases this process held for finished work go
  (`Room::forget_finished_work`); held files of items let in or closed go,
  as before, and so do the empty folders the inbox's files leave (the
  held-files folder, and a received folder, a teammate's or a sender's in
  it, with nothing in it). A file in a received folder is the teammate's and
  stays.
- **Every list is capped**: `read_inbox` lists the newest 50 open items and
  the newest 20 closed or sent; the window is sent the newest 200 closed and
  200 sent (`hotline_store::inbox::VIEW_MOST`); the requests on a pair's
  record go with their items, as above.
- **Nothing a newer Hotline wrote is touched.** A line this build cannot read
  is kept byte for byte through a tidy and never removed, however old; a
  pair's record it wrote anything in is left whole, and its held files stay; a folder with
  anything in it stays.

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

## Topics

A topic is one ask, its answers and the follow-ups on them, between
teammates: the unit the person reads (On the desk, below). It is a field of
the items, not a record of its own.

- **Each item keeps its topic** (`topic`) when it is made: the topic of the
  item its `reply_to` names, else the send id of the `send` that made it.
  The items one `send` made, one for each teammate it reached, share that
  send id (`sendId`), so a send to several is one topic for its sender;
  each receiver's view holds only its own item and what it sent back, so
  replies stay private to each recipient. An answer (`update_inbox`'s
  `answer`, a work thread's result) takes its question's topic and has no
  send id; a loop-in and Hotline's own notices start topics of their own.
  The word is `topic` because `exchange` already names the work-request
  queue (`session/exchanges/`, `item.request`).
- **An item kept before topics** has neither field, and is never written
  again for it: the store gives it both as it reads it
  (`hotline_store::inbox::topics`, for `inbox.list`, the subscription and
  `inbox.item`). The items one old send made for several (the same sender,
  title and teammates reached, kept within five seconds) share the first
  one's id as their send id; an item's topic is the stored topic of the
  nearest item up its `reply_to` chain that has one, else that chain's
  first item's send id, else its id. A chain whose first item is gone
  starts at the oldest item still kept.
- **A follow-up stays in its topic.** `send`'s `reply_to` names an item the
  teammate received or one it sent (`Room::linked_item`), so a teammate can
  follow up on an answer it got and on its own send nobody has answered
  ("did you see this?"), and either lands in the same topic.
- **The person's follow-up** is a line of their own in their chat with the
  teammate: `session.prompt` (and the phone's `mobile.prompt`, in the main
  conversation only) take `aboutItem`, an item the teammate sent or
  received, which the line is written naming (`user.aboutItem`). The
  teammate is told, before the person's words and never on the line the
  person reads, which topic it is about and with whom, and to send it on
  with `send` and that `reply_to` if it is for the colleague, or answer it
  itself. An item that is not the teammate's is refused, and nothing is
  said. Nothing ever sends as a teammate or writes to a colleague directly.
- **Not built:** an `inbox.topic` command listing one topic whole. The
  window builds topics from the owner's view, which holds both sides; the
  phone and search can have it when they need it.

## Guards

- **The person's word.** Only the person's own lines carry the person's
  authority. A colleague's "the person asked for this" is the colleague's
  claim, and the receiver is told so where it reads an item: `read_inbox`'s
  note, a taken item's delivery and its work thread's brief, the tool text
  and the room skill (`inbox::PERSONS_WORD`).
- **Up in access.** `send` checks `authorize_collaboration`: a receiver
  with no more access than the sender (the whole machine is more than its
  own folders) is reached with no card at all. One with more raises a card
  in the sender's chat, "Goldie wants Huck to do work", "Huck has more
  access than Goldie. If you allow this, Huck does the work with its own
  access: your whole Mac.", with Allow (this item only: the next asks
  again), Always (this direction, until the receiver's access grows; the
  informed consent an item that wants work needs) and Deny (the item is
  dropped unseen, and its sender told). A receiver on another agent (an
  ACP harness) sets its own access, which Hotline cannot see, so asking one
  raises the same card unless the sender has the whole machine, which
  nothing is above; a sender on another agent is not known to have it and
  is asked about too (`consent::asks_the_person`). Its words name the
  agent as its settings do: "Huck runs on Codex, which sets its own access.
  If you allow this, Huck does the work with that access." Always for it
  stands until the receiver's access grows or it moves to a different agent. In Needs you it is one question,
  "Allow Huck to do work for Goldie?". On a call that can answer it, the
  call says it from the wait behind it, never its title: "Goldie wants to
  reach Huck. Say yes to allow it this once, or no to decline." ("I want to
  reach Huck" on a call to Goldie); one only the desk can answer is "Goldie
  needs you for an approval: Goldie wants to reach Huck. Please handle it at
  the desk". An item the card has yet to allow is held, as above. The card stands for any item, whatever it wants: a
  receiver may take an item that wanted an answer into work, and the card
  that let it in stands for that work.
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
  it: a line from the person, to either of them, about anything else does
  not, and leaves the count as it is. Keep going counts afresh. Stop ends
  their outstanding work and holds them apart: `send`, an `answer` and
  taking work into a thread between the two are refused with a plain reason
  until the person next writes to one of them (at the desk or on the
  phone), in any of its threads: its main conversation, a work thread, a
  call, a job's thread, or an answer in the chat to a work thread's post,
  which goes to that thread. That lifts the stop and starts the count
  afresh. Until they pause, any such line starts their count afresh too
  (`Room::heard_from_person`, called wherever the person's line enters a
  thread). The card's keys are Keep going and Stop;
  while it waits it is one of the cards in Needs you, above the composer
  ("Needs you · Paused with Huck"), and after Stop it leaves one quiet line,
  "Stopped. Write to either of them to start again." Their open items stay
  open for the receiver to close or leave. The result of a work thread is
  counted as a request's reply always was.
- **The hourly limit** (task-472, `inbox/limit.rs`). Turns nobody watches
  have a ceiling per teammate: one whose main conversation has begun as many
  quiet turns in the last hour as the person allows (Settings › General ›
  Conversations, "Pause a teammate that works [20] times an hour without
  you", the room setting `quietTurnsPerHour`, 1 to 1000; 20 is a placeholder
  until desks say how many a teammate needs) is paused before the next, with
  the pair's card for one teammate (`quiet_paused`): "Paused", "Huck has
  worked 20 times in the last hour on teammates' messages, without you.",
  Keep going and Stop; in Needs you, "Huck has worked 20 times this hour
  without you" (once, for one). "Quiet turn" is the desk's word, never the
  person's: they read how many times it worked without them. While it waits
  no quiet turn begins (no wake, no pick-up): its items still land, and a
  turn the person starts hears them. Keep going counts afresh and wakes it
  for what waited ("Kept going"); Stop holds it until the person next writes
  to it, in any of its threads, which counts afresh ("Stopped. Write to Huck
  to start again."); while it is not paused, any such line counts afresh
  too. A
  quiet turn is one the inbox begins in the main conversation (a new item's
  wake, a notice the driver did not take mid-turn, a pick-up); a work thread
  or a job the inbox wakes is not counted. The count is the process's, so a
  restart counts afresh; the pause and the stop are on the room stream
  (`quiet_limit`) and hold across one.
- **How often a brake fires.** Each pause, a pair's or the hourly limit's,
  adds one line to `brakes.jsonl` in the data directory: `{"at", "brake":
  "pair", "teammates": [a, b], "count", "limit"}` or `{"at", "brake":
  "quiet", "teammate", "count", "limit"}`, so the numbers can be set from
  what happens on a desk. Nothing reads it but a person.
- **One path.** Nothing reaches a teammate by `ask_in_main`,
  `ask_in_own_thread`, a sender-triggered `dispatch_handoff`, or
  `aside_from`/nudge: those are deleted. An ask from before the inbox that is
  still open when this version starts is ended as unanswered and its sender
  told by Hotline's notice in its inbox (`notice: unanswered`), never in the
  colleague's voice, to send it again. A handoff from before the inbox runs
  on as it did, a follow-up in the thread it continues, and what it comes to
  is a turn of the thread that sent it, as in 0.49
  ([sessions.md](../sessions.md#ask-or-hand-off), Saved before the inbox).

## What was removed

`message_teammate` and `aside` (tools, handlers, glance labels and guidance);
`Room::send_intent` and the request queueing behind it; asks routed to a main
conversation, to a work thread of their own, back to a delegator, or into a
named thread (`exchanges/questions.rs`), and the work thread's brief for a
question; asks as gates on a handoff (`gate_on_ask`); continuing a handoff in
a named thread (`handoffThreads` in `list_teammates`; a follow-up saved
before the inbox still continues its thread); the per-turn note and result nudge a handed-over
thread left its opener, and the note of the person's Stop or End thread (the
item's receipts say it now); lines on the pair's thread; a colleague's aside
and its crossing to a linked desk; the linked desk's requests, notes and asides
arriving here, and files crossing a link.

## Not in 0.50

- **Linked desks.** Messages between teammates cannot yet cross to a
  teammate on a linked desk: 0.49 could pass a message to another desk, and
  0.50 does not. `send` refuses a teammate on a linked desk with a sentence,
  and a linked desk's request or aside is refused at `federation.deliver`.
  The link still carries rosters, and the answer to a request an older
  version forwarded. A message from another desk will be a message from
  outside, with the protections inbound sources have, when it crosses.
- **Inbound sources** stay on their aside path (`inbound.rs`, `inbound_aside`),
  with their own grant, level and brake; turning them into inbox items would
  change the source grant model.
- **A work thread waiting on a reply.** A taken item's thread that `send`s
  with `waiting: true` still ends its turn and returns its result; the reply
  is heard in that thread only while it is live. Asks used to hold a
  handoff's result back as a gate.
- **Mid-turn notices in work threads**, jobs and runs: a busy one hears its
  items at its next turn's start.
- **Relevance**: no score or gate decides which items notify.

## On the desk

The window draws the inbox from the contract below (task-456), as topics
(task-514, the approved inbox-mail drawings). The person reads; the
teammates act: nothing on the desk sends, sorts, answers or marks an item
seen, and no chat opens between teammates. Wherever the person reads it, an
item in `taken` reads Picked up (its state word, its receipts, the inbox
pane's group and count line, the sent rows and the conversation's rows); the
stored state and the teammates' tools keep `taken`.

- **Sent rows.** Each `inbox_sent` is a quiet row like a thread's link: the
  tray, the title, whom it went to and where it stands now ("To Juno ·
  Picked up"). The items one `send` made to several teammates under one title
  stand side by side on the tape and are one row, which says where they
  stand answers first ("To Juno, Ivy, Huck · 1 done · 1 seen · 1 awaiting
  approval", `groupWords`). The state is read live from the
  sender's `{"inbox"}` subscription, not from the line. A press opens the
  topic its item is in, which for a send to several is the whole send,
  every teammate's lane, in the right-hand pane (`ui/src/inbox.ts`,
  `components/transcript/InboxLines.tsx`). In a work thread a sent row is
  its own row, the title in bold and the tray in the accent; in the
  teammate's main conversation it is in a run (below), or, sent in a turn
  the person asked for, a quiet row of its own: the title in plain weight,
  the tray in grey, its state word kept, since it is what came of their ask.
  So it is the turn's receipt (task-505): a turn whose only work was the
  inbox (`send`, `read_inbox`, `update_inbox`; saying what came of it, a
  reaction and a post are no work, nor is what reaching a colleague takes
  on the way: a look at the team, Claude Code loading its tools, Hotline
  Agent reading the room's skill) draws no outcome line under a reply
  while one of its sent rows is drawn, only its time, and a turn the person
  asked for that also put no bubble in the chat draws no time either: the
  row stands for the turn (`sentRowsSay` in `ui/src/scheduledRuns.ts`). An
  outcome that failed or only partly happened keeps the line. It is the
  window's drawing: the tape, the wire and the phone's line are unchanged.
- **A run of inbox activity.** In the teammate's main conversation, its
  inbox activity between two things the person should read is one quiet
  line, "Inbox · read 3, sent 1", drawn as a line of a turn's path is (the
  dot, the words, the chevron; `FoldLine` in
  `components/transcript/Trail.tsx`, which the path's "Started 4 helpers"
  line also is), with no card, header or numbering (`foldInboxRuns` in
  `ui/src/scheduledRuns.ts`, `InboxRun`). It says only what the run has:
  how many items it sorted ("read 2") and how many sends it made ("sent
  1"). Pressed, it opens in place, with no rule or box around them, to the
  run's rows in the order they happened, each a quiet row (plain weight,
  the tray in grey): a row per item a sort moved (each `inbox_sorted`
  entry: its title, "From" its sender, Hotline for the desk's own notice,
  what became of it in the word the sent row and the panes use, and a
  later's reason), each send, the work thread an item started once it is
  over, and the time of each quiet turn ("5.0s ›"), which opens that turn's
  path in the pane as the line under a reply does. A press on an item's row
  opens the item.
  - **What ends a run:** the person's own line, a bubble the teammate
    posts, a turn the person asked for (a sort in one is a run of its own
    above the reply, its sends stay out as quiet rows), and any other row:
    a card, a notice, a chapter, a call, a scheduled line.
  - **What stands beside it:** a turn's end, which draws nothing; a quiet
    turn still running; and a work thread an item started while it is live
    or parked, which does not split the run but is drawn under its line,
    and joins the line once it is over.
  - **A quiet turn that posted** keeps its time under its bubble, as any
    reply's is, and its own sorts and sends are no part of a run
    (task-505): its bubble says what came of them, so they draw no line,
    and its path lists them under Inbox, before its steps, each a row
    that opens its item (`turnInbox` in `ui/src/inbox.ts`, `TurnInbox` in
    `components/InboxPane.tsx`). A quiet turn that posts nothing keeps its
    line. Every turn's path has the section when it sorted or sent.
  - **Growing.** The line stands where the run's first row is and is named
    by it, so as the run grows it keeps its place and stays open or shut;
    only its counts and rows change. A run that read and sent nothing (a
    quiet turn's time alone, a finished work thread alone) is drawn as its
    rows were, with no line.
  - It is the window's fold of the tape: no wire field changes, and the
    phone, which draws no inbox rows, is not touched.
- **A quiet turn.** The turn the inbox woke draws as path, not as a reply:
  its sorts, its sends and its time are rows of the run it is in, its tool
  rows and its words, as thinking, in the turn's steps behind that time,
  and no bubble unless it posted. A sort in the middle of a turn the person
  started is a run of its own where it fell, above the reply; it does not
  split the turn's steps, which stay one run under the reply. A turn that
  ends without a new line in the chat posts no desktop toast
  (`ui/src/notify.ts`).
- **Woken by its inbox.** A bubble the teammate posts from a quiet turn one
  item woke carries one quiet line above it, "From Goldie's message · Rerun
  the Linux build" (the sender's name, the item's title; "From Hotline's
  notice" for the desk's own), a plain row with the tray in grey, which
  opens the item. The desk stamps the post with the item (`fromItem`) when
  exactly one item woke the turn: its line or its index offered that item
  alone; several together could each be what it posts about, so it names
  none. A bubble straight under the sent row of the item it answers draws
  no such line (task-505): the row above says what was asked and of whom.
  Straight under means nothing is drawn between them but a time stamp and
  the lines the window leaves out (an inbox-only turn's outcome and time, a
  posting quiet turn's inbox line); the answer's own bubbles count as one.
  An answer the window cannot link to a row in view (its item names none it
  answers, or the row is not loaded) keeps the line, and so does every
  file, card and job (`answersUnderTheirRow` in `ui/src/scheduledRuns.ts`).
  The turn's end is marked `quiet` when the person never spoke into
  it, and the steps pane heads such a turn "Worked quietly", never "Done",
  whatever turn is working now.
- **What a colleague's message prompted** (task-469). The same line, by the
  same rule (`Room::prompted_by`), is over everything else such a turn makes
  that the person sees, so a card that appears because a colleague asked
  for it never looks like it came from the person's own request: a file it
  sends (`send_file`, `generate_image`: the post carries `fromItem`); a card
  it raises, a `request_human` or `ask_question` card (a source proposal, a
  new teammate or the person's name included), a credential request, a
  sign-in card (`ask_cookies`), a permission its harness asks, and the card
  up in access its `send` raises ("Goldie wants Huck to do work"), each
  drawn under the line in the conversation and in the composer's Needs you,
  and kept on the card once it is answered; and a job it makes with
  `schedule` or `loop`, which records the item (`fromItem`) and whose row in
  Schedules has the line under it, or "From a colleague's message" once the
  inbox no longer keeps it. Not covered: a turn several items woke, a quiet
  turn the person spoke into (it is theirs from then), a work thread taken
  from an item (its pane is headed by the item as its Message), a job's run
  woken by a reply, the browser's own cards (a local address, a secret's
  use, a passkey: raised by the browser, which cannot tell which of the
  teammate's turns asked), a permission card's expired afterlife (its
  harness rewrites it without the line), and a first-contact card raised
  again after a restart.
- **A topic**, on the pane's skeleton (`ui/design.md`, "The right-hand
  pane's skeleton"; `components/InboxTopic.tsx`), opened from a row, a
  sent row or anything that names one of its items. Its title, then its
  state line: where it stands (a send to several answers first; Awaiting
  approval, in amber, while something in it waits for the person; else the
  newest open item to the owner, else the newest it sent, else how it
  ended), who waits ("Huck is waiting") or whom it is with, and when it last
  moved. Then its messages in order, read as a conversation: the inbox
  owner's on the right in the raised container, the other side's on the
  left in the quiet container old conversations use, never the person's
  mint. A message is a bubble when it has no words beyond its title, or its
  words are one plain paragraph of at most 160 characters with no list,
  heading, quote, code or file; anything else is a letter, a container with
  the sender and time, its own title only when it is not the topic's or a
  "Re:", six lines and Show all (`isShort`). Under each, its time and one
  word for where it stands (Sent, Seen, Picked up, Later, Answered, Done,
  Expired…), which opens that message's history in place; Awaiting approval
  is always written out, in amber, with "Go to the card", which opens the
  first-contact card in the composer's Needs you, where the person answers
  it. The work thread a message started is a row under it, live while it
  runs, which opens the thread. A send to several is the send, "To Huck,
  Quill, Juno", then a lane for each teammate, answers first, each with its
  state on its rule and its own replies, "No reply yet.", or "Waiting for
  your approval. Go to the card". At the foot, Ask a follow-up (below). A
  time from before today gives its date. Hotline's notice is from Hotline,
  by its mark, wherever an item's sender is named. Back goes where it was
  opened from (`DockState.from`). An item older than the newest the view
  is sent is read by id with its replies (`inbox.item`).
- **Ask a follow-up** fills the message box of the chat beside the pane,
  "Following up on What was the score? · Timo", as replying to a bubble
  does; nothing is sent until the person sends it, and then it is their own
  line with `aboutItem`, the topic's newest message (Topics, above). The
  person's bubble carries the quote, whom the topic is with and its title,
  which opens the topic. It is the person's own message box only: the desk
  never sends as a teammate.
- **One word for each outcome.** An item a reply went back on is Answered
  and one whose work finished is Done, the same word on its sent row, its
  sort's row, the inbox pane and the topic. Noticed, the desk showing
  the receiver a title, is the desk's own step: an item in it reads as Sent,
  and the receipts leave it out. In the Closed list a finished item is
  ticked, a dropped one crossed, and an expired one, never picked up, keeps
  a dotted ring.
- **The inbox pane.** A search field (every message of every topic, open
  and closed, and who is in it; the tabs hide while it shows what matched),
  then two tabs. Open groups topics To sort (an item to the teammate is new,
  noticed or seen), Picked up (one is taken), Waiting on others (something
  it sent is open or held), For later (one is set aside); everything else is
  Closed. Each group is headed on a rule with its count; there is no count
  line, and no Sent tab: what a teammate sent waits under Waiting on others
  until it is answered or done. One row per topic, newest first: its mark,
  its title, who said what last ("Timo: Devils 7–4."), or where a send
  stands ("Huck · Seen"; "Juno · Awaiting approval" in amber; answers first
  for several), and when it last moved, with the live dot while its work
  runs. A topic with something the teammate has not read is bold with its
  ring in the accent; a seen one is in To sort with a half ring and no part
  of the unread count. In Closed a finished topic is ticked, a dropped one
  crossed, and an expired one keeps a dotted ring. It opens from the
  teammate's menu (Inbox, beside Threads, with "N unread") and from the
  composer's tray.
- **Work an item started.** The desk opens a taken item's work thread with
  the item's sender as its opener, as a handoff was. The window tells the two
  apart by the inbox views (`item.work`): such a thread is a work thread
  everywhere. On the receiver's conversation it is a link row with the live
  dot, its title and "Work thread · Running" (or Open, Parked, or what came
  of it), and on the sender's "Work thread · Juno · Running"; the Threads
  list and the composer's count list it as a work thread; its pane is the
  work thread's, with the item's words over it as its Message, whose Open ›
  opens the item (the one taken, not one combined into it), with Back to the
  thread. A handoff from before the inbox keeps its old line and pane.
- **The composer and the rail.** While the open teammate has unread items,
  the composer's bottom row has "N unread" with the tray, beside "N threads"
  and in its style; it opens a tray of them (title, sender with "waiting"
  when the sender waits, and when each arrived, the same on every row) with
  a last row to the whole inbox. The rail shows no inbox count: a
  teammate's unread is its own business, and the chip says it the moment
  the person opens that conversation.
- **Conversations from before the inbox.** A conversation two teammates had
  in 0.49 (a pair thread) is kept to read: in the Threads list it is always
  under Earlier ("Earlier and routine" under Active), however recent, named
  "Conversation with Goldie" for the other teammate, and its pane
  (`PairPane`), Conversation in its band and the other teammate's name its
  title, has no composer and no key that writes to it, stops it or starts it
  again: a pause card left waiting in it is a plain line saying it was never
  answered, with no Keep going or Stop. It is never live: the work a receiver takes from its
  inbox runs on the pair's record but in the receiver's work thread, so the
  pair's thread is always `Parked` and no roster row carries it. Nothing
  starts a new one: no tool writes to a pair (`send` is the one way between
  teammates), and the wire refuses a line, a Loop in or an End thread on one.
- **What was removed.** The teammate pane's Threads section, which listed a
  teammate's conversations with colleagues; a pair's Stop exchange and its
  live state (`workingPairs` on the roster row). Colleague asides still on
  disk keep rendering where they always did (the Threads list's From
  teammates, `AsideLine`), and inbound sources' and Loop in's asides still
  draw as asides.

## Contract

Types (`crates/hotline-contract/src/inbox.rs`, generated into
`ui/src/generated/contract.ts`):

- `InboxItem` as above; `InboxReceipt {state, at, reason?, outcome?, into?}`;
  `InboxState` `new | noticed | seen | taken | later | closed`; `InboxOutcome`
  `answered | done | noted | dropped | combined | expired`; `InboxNotice`
  `expired | unanswered`;
  `InboxWants` `answer | work | nothing`. `InboxItem.notice?` marks Hotline's
  own notice, `InboxItem.offerAgain?` is when a pick-up offers it next,
  `InboxItem.forPerson?` marks one sent in a turn the person read, and
  `InboxItem.topic?` and `InboxItem.sendId?` are its topic and send
  (Topics, above), absent on an item kept before them and filled in by the
  store as the view and `inbox.item` read it.
- `InboxView {personaId, open, closed, sent, unread, newer?}`: open and
  closed newest first, what the teammate sent others newest first (the
  newest 200 closed and 200 sent), its unread count, and `newer`, the items a
  newer Hotline saved that this build cannot read, as `InboxNewerItem {id,
  ts, from, to, title, state}`.
- `InboxItemDetail {item, replies}`: the item with its receipts and its `work`
  thread, and the items that answer it.
- `RosterEntry.inbox?`: how many open items the teammate has not read, which
  the teammate's menu says beside Inbox.

Wire commands ([wire.md](../wire.md)):

| Command | Params | Result |
| --- | --- | --- |
| `inbox.list` | `{personaId}` | `InboxView` |
| `inbox.item` | `{id}` | `InboxItemDetail` |

`session.prompt` and `mobile.prompt` take `aboutItem?`, the item the
person's line follows up on (Topics, above).

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
- `user.aboutItem?`: on the person's line that follows up on an inbox
  topic, the item it follows up on.
- `agent.fromItem?`: on a post from a quiet turn one item woke, that item,
  and the same on a `permission`, `human_action`, `credential_ask` and
  `cookie_ask` card raised in such a turn; `turn.quiet?`: on the end of a
  turn quiet to its end. Each is an added field a reader that does not know
  it passes over, so a tape stays the reference tree's file.
- `ScheduledJob.fromItem?` and `ScheduleEntry.fromItem?`: on a job such a
  turn made, the item.
