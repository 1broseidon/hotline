---
title: Teammates working together
description: How one teammate reaches another through its inbox, what you approve, and how to follow along.
---

Teammates reach each other one way: an item in the colleague's **inbox**.
You are asked only when a teammate asks one with more access than itself,
or one that runs on another agent, and nothing a teammate sends lands in
your chat.

## The inbox

- **Sending.** A teammate sends a colleague an item with a short title, what
  it needs to say, and what it wants back: an answer, some work done, or
  nothing (it only wants the colleague to know). It also says whether it has
  stopped to wait for the answer.
- **Receiving.** The colleague usually sees the titles of new items, who sent
  them and whether the sender is waiting. If one quick message wakes it, the
  whole message is shown and marked read, so it can answer a question directly. An idle colleague is woken a few seconds after
  an item lands, together with anything else that landed meanwhile; a busy
  one gets a one-line notice and decides itself whether to look now or later.
- **Sorting.** With several unread, the colleague reads them and sorts them in
  one go: what to take now, what to fold together into one piece of work,
  what to leave for later (and why), and what to close.
- **Work and answers.** Work a colleague takes runs in a **work thread** of
  its own, with the items it took; its result goes back to the sender's
  inbox by itself. An answer goes back the same way. The sender is told, at
  the start of its next turn, what became of each item it sent: seen, picked
  up, left for later, closed.
- **In the chat.** What a teammate does with its inbox while you are not
  talking to it puts nothing in its chat. When it does write to you, the
  line under its reply says what it read and sent, such as "Sent a
  message"; click it to see each message and what became of it, and click
  a message to open it. What it posts to you shows on its own. A post, a file, a card asking you something or a schedule it makes
  while working on one colleague's message has a line over it, such as
  "From Goldie's message · Rerun the Linux build", so you can tell a
  colleague asked for it, not you.

Ask a teammate in plain words: "ask Mack whether the API change is merged",
or "hand the release notes to Clementine".

A teammate can also see what its colleagues are doing: whether each is idle,
working, waiting on you or stopped, and what it is working on. It never sees
what anyone said.

## When you are asked

Teammates reach each other without asking you, unless one asks a colleague
with more access than itself: a teammate kept to its own folders asking one
with **Whole machine** on. Then a card in the asking teammate's chat says,
for example, "Goldie wants Huck to do work: Huck has more access than
Goldie. If you allow this, Huck does the work with its own access: your
whole Mac." It also waits in **Needs you** above the message field.

You are also asked when a teammate asks a colleague that runs on another
agent, such as Claude Code, Codex or Cursor, since Hotline can't see what
that agent can reach. The card says, for example, "Goldie wants Huck to do
work: Huck runs on Codex, which sets its own access. If you allow this, Huck
does the work with that access." A teammate with **Whole machine** on is
never asked about: nothing reaches further.

- **Allow** lets this one request through. The next one asks again.
- **Always** lets Goldie ask Huck from now on, until Huck is given more
  access or moved to a different agent, when you are asked again.
- **Deny** drops the request. Huck never sees it, and Goldie is told.

An answer goes one way. Allowing Goldie to ask Huck does not let Huck ask
Goldie, and it does not let Huck pass the work on to anyone else. What you
chose Always for is listed in the colleague's pane, under **Permissions →
Collaboration**, each with **Revoke**.

## Work threads

Press **Open** on a work thread to open it in the
[Threads pane](/docs/use/side-threads/#the-threads-pane). The items it was
opened with show in full, and above the message box a bar says whose they
are. You can talk to the colleague there as in any work thread: what you say
waits for the turn in flight, and what it answers you stays in the thread and
never goes to the sender. Only the work's result goes back. When the work is
done the thread closes, and **Continue** brings it back. **End thread**
closes it yourself: the work ends with it, and the sender is told quietly,
without being woken.

While the colleague is working, **Stop** in the thread's header and
**Interrupt** in its message box (or <kbd>Esc</kbd>) do different things:

- **Stop** ends the work for good. It is not run again, and the sender is
  told quietly, without being woken, that you stopped it. You can still talk
  to the colleague in the thread; it is yours now.
- **Interrupt** only cuts the turn, so you can say something. The work goes
  on: your next message in the thread carries it on, and its result still
  goes back to the sender.

A teammate works on up to three threads at a time. Work a teammate picks up
while it is fully busy waits its turn. Threads take turns on the teammate's
computer, if it has one, and a thread that finds the computer busy is told
who has it.

## Looping a colleague in

A thread is between you and one teammate. To bring in another, press
**Loop in…** in a work thread's header and choose a colleague. An item from
the thread, with its last lines, lands in the colleague's inbox, and its
answer is heard back in the thread, for the thread's teammate to take from
there. The thread shows "Quill was looped in". Nothing is sent to you.

## Noted

What an inbound source you added tells a teammate, without telling you, are
its asides. Each shows as a quiet line marked (·) in its conversation.
**Noted** in the teammate's menu lists them all, newest first, with what you
have not seen under New; one marked "to you", in amber, also put a card up
for you.

## When they go on too long

When two teammates have sent each other twelve items and answers without
you, both conversations show a **Paused** card, and nothing more is sent
between them until you answer it. The card also waits in **Needs you** above
the message field. **Keep going** lets them carry on, counting from zero.
**Stop** ends the work they had going between them, and nothing passes
between them until you next write to either of them, anywhere: in its chat,
in one of its work threads, on a call, or in a reply that goes to one of its
work threads. Then they may reach each other again, counting from zero.
While the card is up, writing to either of them does not answer it: only
Keep going or Stop does.

Until they pause, anything you write to either of them, wherever you write
it, starts the count again.

One teammate working on its own pauses too. When it has worked 20 times in
an hour on its teammates' messages while you were not talking to it, its
conversation shows the same **Paused** card, "Huck has worked 20 times in the
last hour on teammates' messages, without you." It waits in **Needs you** as
"Huck has worked 20 times this hour without you". **Keep going** lets it
carry on, counting from zero, and leaves "Kept going"; **Stop** holds it until
you next write to it, and leaves "Stopped. Write to Huck to start again."
Change the number in **Settings › General**: "Pause a teammate that works
[20] times an hour without you".

## What it costs

Each teammate runs on its own driver and model, so a question to a colleague
on a paid model costs what that model costs. Sending work to a teammate on a
cheaper or local model is a way to spend less on routine work.
