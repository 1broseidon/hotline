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

- **`send`** `to` (one teammate, several, or `["team"]`), `title` (one line,
  at most 80 characters; a longer one is cut at a word), `message` (optional),
  `wants` (`answer`, `work` or `nothing`), `waiting` (the sender has stopped
  until this is answered), `about` (a thread of the receiver's, `kind:key`),
  `reply_to` (an item in the sender's own inbox), `files`. It returns at once
  with each item's id. `wants` and `waiting` are described to agents as facts
  about the sender, never as urgency.
- **`read_inbox`** without `ids` lists the open items, newest first: id, title,
  sender, `wants`, `waiting`, state and age, never the words. With `ids` (any
  unique prefix of four characters or more) it returns them whole and marks
  them seen.
- **`update_inbox`** `items`, a list in the order the receiver means to work
  them. Each entry names an item and sets `state`: `taken`, `later` (with a
  `reason`), `closed` (with an `outcome`: `done`, `noted` or `dropped`) or
  `combined` (with `into`, the item it folds into). An entry may carry an
  `answer`, which replies and closes the item as done. `in_thread` (with
  `taken`) says whether the work runs in a work thread of the receiver's: true
  by default for an item that wants work, false otherwise. Every entry is
  checked before any moves, so one mistake changes nothing.

`list_teammates`, `post_to_chat` and the rest are unchanged. The lead's voice
front on a desk call has `send` in place of `message_teammate`, for named
work only, by the call's grant.

## An item and its states

An item (`InboxItem`) has an id, when it was sent, who sent it and from which
of its threads (`from`, `fromThread`), whom it is for (`to`), the title, the
words (`body`, with where any files landed), `wants`, `waiting`, `replyTo`,
`about`, its `state`, and every state it has been in with its time
(`receipts`). Once taken into a thread it names the thread (`work`) and the
exchange request the work runs as (`request`).

```
new ──▶ noticed ──▶ seen ──▶ taken ──▶ closed (done | noted | dropped | combined)
  │        │          │        ▲
  └────────┴──────────┴──▶ later ──┘
```

- `new`: it landed. `noticed`: its title was put in front of the receiver,
  at a turn's start or in a mid-turn notice. `seen`: `read_inbox` returned
  its words.
- `taken`, `later` (with the reason) and `closed` (with the outcome, and for
  `combined` the item folded into) are the receiver's sort. A `later` item can
  be taken or closed afterwards; a closed one is over.
- An item that the person's first-contact card has yet to allow is kept
  `held`: the receiver cannot read it, and it does not count as unread. A yes
  lets it in as `new`; a no, an expiry or the person's Stop exchange closes it
  `dropped`, with why, unseen.

Items are kept on the receiver's own stream, `inbox/<personaId>.jsonl`
(`StreamId::Inbox`), one line per item, written again under its id at each
change: the fold is the whole state, and a restart reads it back as it was.
At start-up a held item waits on its card again, and an item no turn has been
offered wakes its receiver.

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
- **Busy.** An item that lands mid-turn starts no turn and no work thread.
  When the gather delay is up, the turn in flight in the main conversation is
  handed the one-line notice, steered into it as the person's mid-turn line
  is, at most once per gather window: `HOTLINE_INBOX_MIDTURN=0` (or `off`,
  `false`) turns this off. A driver that takes no line mid-turn hears the
  notice as its next line instead. A busy work thread hears its items at its
  next turn's start; it is not handed a notice.
- **At a turn's start.** Every turn of the main conversation, a work thread
  or a call opens with an `INBOX` section in its thread index: the count of
  unread items and items for later, the new items not yet offered to a turn
  (title, sender, `waiting on you`, what it wants, age, short id, and which
  item it answers), and what moved among the items the teammate sent. An item
  is offered once; the count stays while it is unread. The section is fenced
  as data (`hotline_inbox`).
- **At a turn's end.** Items still unread that no turn's start was offered,
  because they landed mid-turn, wake the idle thread as above. An item wakes
  its receiver at most once: one offered and left unread waits for the
  receiver to come back to it.

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
when it was sent.

An `answer` on `update_inbox`, and the result of a work thread a taken item
started, are new items in the original sender's inbox with `replyTo` set,
`wants: nothing`. The original closes as `done` (as `dropped`, with the
failure, when the work failed). A result goes to each sender whose item the
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
`"Pick a name" to Bob · later 14:03: after the release · 1a2b3c4d`. Only the
changes, like the index. The window reads the receipts over the wire.

## Guards

- **First contact.** `send` checks `authorize_collaboration` as an ask or an
  aside was checked, and for an item that wants work as a handoff was
  (informed consent). An item the card has yet to allow is held, as above.
- **The brake.** Every item and every reply counts on the two teammates'
  `EXCHANGE_CAP` (12); at the cap they pause, the person has the card, and
  nothing more is sent or answered between them until the person says to
  keep going. The result of a work thread is counted as a request's reply
  always was.
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
- **The sorting card.** Each `inbox_sorted` on the receiver's main
  conversation is one raised card: "Read 4 messages", the time, and the
  counts (taken, for later, closed); then the entries in the order given,
  taken and later ones numbered, a combined one set in under the one it was
  folded into, a noted or dropped one marked "–". Each says its title, its
  sender ("waiting" where the item says the sender stopped for it), any
  reason, and its state at the right, Taken in the accent. A sort of one
  item is a quiet row instead. A press opens the item.
- **The item pane.** Its title; from whom to whom, what it wants, and
  whether the sender waits; the words; Receipts, a timeline of sent,
  noticed, seen, taken, later with its reason, closed with its outcome (and
  "Allowed" when the first-contact card let a held item in); the work thread
  taking it started, which opens that thread's pane (beside the receiver's
  conversation when the open teammate's Threads list does not hold it); the
  item it answers and the items that answer it. A held item says it waits on
  the first-contact card. Back goes where it was opened from.
- **The inbox pane.** Open (Unread: new, noticed and seen; Taken; For
  later), Closed and Sent, newest first, under a line of the open counts. An
  unsorted item whose sender waits counts how long in amber. It opens from
  the teammate's menu (Inbox, beside Threads, with "N unread") and from the
  composer's tray.
- **The composer and the rail.** While the open teammate has unread items,
  the composer's bottom row has "N unread" with the tray, beside "N threads"
  and in its style; it opens a tray of them (title, sender, amber age when
  the sender waits) with a last row to the whole inbox. A rail row shows the
  teammate's `inbox` count while it is above zero.
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
  `done | noted | dropped | combined`; `InboxWants` `answer | work | nothing`.
- `InboxView {personaId, open, closed, sent, unread}`: open and closed newest
  first, what the teammate sent others newest first, and its unread count.
- `InboxItemDetail {item, replies}`: the item with its receipts and its `work`
  thread, and the items that answer it.
- `RosterEntry.inbox?`: how many open items the teammate has not read.

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
  answered?}` in the order given, and how many items are open and unread
  after it.
