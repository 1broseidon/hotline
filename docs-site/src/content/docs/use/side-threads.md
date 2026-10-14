---
title: Side threads and subagents
description: Talk to a busy teammate about something else, and follow the workers it sends off.
---

A teammate can work on more than one thing at a time. A **side thread** is a
second conversation with the same teammate that you start. A **subagent** is
a worker the teammate starts. Both open in the **Threads** pane on the right.

## Side threads

Your teammate is in the middle of a long build, and you want to ask it
about something else without derailing it. Choose **More → Start a side
thread** ("Another topic with" the teammate's name, "in parallel").

The thread opens at once in the right-hand pane, empty. The first thing you
say in it becomes its title. It is the same teammate, with the same folders,
tools and permissions, but it runs in a context of its own. A new side
thread is told the main conversation's last handoff note and its last few
lines, so it knows what the teammate is busy with. After that, nothing said
in the side thread reaches the main conversation, and the other way round.

While a side thread is running, a chip in the conversation's header shows it
("Side:" and its title, or a count with a menu when there are several), and
the main conversation has one line for it with **Open**.

A side thread lasts as long as the topic does:

- **The teammate never closes it by itself.** It is archived when you tell
  the teammate you are done, or press **Archive** in the thread.
- **A quiet thread is parked.** After three quiet hours, or a restart, no
  agent is running for it, but it stays in the list. Saying something in it
  picks it back up where it left off.
- **An archived thread can come back.** Open it and press **Continue**.

When a side thread is archived, the main conversation keeps its title and a
one-line outcome. A closing note (the goal, what got done, what is still
open, the key files) is written where the teammate's own search can find it,
so the main conversation can ask about it later.

A teammate runs at most three threads at a time. Starting another parks the
one you have left alone longest. Parked and archived threads do not count.

## Subagents

A teammate can hand a piece of its own work to a **subagent**: a fresh worker
that runs as the teammate, in the same folder, with the same tools and
model, but with none of the conversation. The teammate keeps talking to you
while the subagent works, and tells you what it found when the report comes
back.

While one is running, it shows at the top right of the message box, by its
title, or as a count with a menu when there are several. Press it to read
what the subagent is doing. Each subagent also leaves one line in the
conversation with its result, and that line opens the run too. Runs are
read-only. Stopping a Hotline Agent teammate stops its subagents.

Hotline Agent teammates start subagents with a tool of their own, up to four
at a time. Claude Code and Codex show their own subagents the same way when
their adapter reports them; other harnesses show the work as steps of the
teammate's turn.

## The Threads pane

**More → Threads** opens the pane on its list. It shows everything running
beside the open teammate's main conversation:

- its side threads, and work threads other teammates
  [handed it](/docs/use/teammates-together/#handoffs), first;
- its conversations with other teammates (**Pair**);
- subagent runs (**Subagent**);
- calls (**Call**).

Each row shows whether it is running, waiting on you, parked or closed, with
its last line. Closed threads fold away at the bottom. A row opens the thread
inside the pane, with **Back to threads** at the top. A permission card in
any thread is answered where it is.

Drag the pane's edge to resize it. In a narrow window, the pane lies over the
conversation. <kbd>Esc</kbd> steps back from a thread to the list, then
closes the pane.
