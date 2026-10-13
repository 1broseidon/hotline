---
title: Common workflows
description: Ways people put a team of agents to work, step by step, with the settings each one needs.
---

Each of these is a task you might bring to Hotline, with the settings it
needs. They are starting points; mix them.

## Put a coding agent you already use in the room

You pay for Claude Code or Codex already and want it as a teammate.

1. Make sure its CLI runs in a terminal on this machine (`claude`, `codex`,
   `cursor-agent`, `opencode`, `gemini` or `grok`), signed in.
2. **New teammate**, then under **Thinks with → Agent** pick it from the
   agents you already use. A row that cannot start says what is missing.
3. Give it the project folder as its working directory.

It keeps its own login, tools and permission modes. Hotline adds the room:
the conversation, schedules, its colleagues, a computer, your phone. See
[Teammates and drivers](/docs/setup/teammates/).

## Hand over a long job and walk away

1. Tell the teammate the job and what "done" looks like: "upgrade the
   project to the new framework version, get the tests passing, and tell me
   what you could not fix."
2. Leave. Closing the window does not stop it; the teammate keeps working.
3. A toast, or a notification on your [phone](/docs/phone/), tells you
   when it finishes or needs you.

If it needs something only a person can do, such as a login or a decision,
a **Needs you** card waits in the conversation. Its turn does not wait for
you: it carries on with what it can, and your answer arrives as a new
message.

## Ask about something else while it works

The teammate is busy and you have a quick, unrelated question. **More →
Start a side thread** and ask there. The main work is not interrupted and
the side thread does not fill its context. See
[Side threads](/docs/use/side-threads/).

If your message *is* about the work in hand, such as a correction, just send
it in the main conversation. It reaches the teammate mid-turn.

## A morning report

Have a teammate check something every day and speak up only when there is
something to say.

1. Open its pane (<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>I</kbd>), and under
   **Schedules** choose **Add a schedule**. Pick **Repeating**, every **1
   days**, and under **Say** write a prompt such as "Check the overnight CI
   runs and open issues labelled urgent. Tell me only what needs me."
2. Tick **Quiet** for the job. A quiet run stays out of the conversation
   unless it finds something worth telling you; a Hotline Agent teammate then
   tells you as a normal reply and your phone is notified.

Hotline has to be running when the schedule is due. See
[Schedules](/docs/setup/schedules/).

## A second pair of eyes

Two teammates, one that writes and one that reviews.

1. Make a **reviewer** teammate with the same project folder and a goal such
   as "Review changes for correctness and say what you would block on."
   Putting it on a different model from the writer is useful: it misses
   different things.
2. Tell the writer: "when you have a change ready, ask the reviewer to look
   at it."
3. The first time, approve the pair on the card that appears.

The review arrives in the writer's conversation, and the exchange is kept
as a conversation between the two. See
[Teammates working together](/docs/use/teammates-together/).

## Split work across specialists

Give each teammate one area: a backend teammate in the server repository, a
frontend one in the web app, a docs one. Tell one of them to hand pieces to
the others. Each handoff opens a work thread on the receiving teammate, and
the result comes back to the sender. You can keep talking to every teammate
while it works on what it was handed.

## Let a teammate use the web for you

1. In the teammate's pane, turn on **Computer**. The first start downloads the
   desktop image. See [The computer](/docs/setup/computer/).
2. Ask: "sign up for the staging dashboard and screenshot the usage page."
3. When it reaches a login it cannot do, a **Needs you** card opens the
   computer's screen. Type the password yourself; it goes to the desktop,
   never through the agent. Mark the card done.

For a quick lookup, the computer is not needed: every teammate can
search the web with `web_search`.

## Work across two repositories

A teammate that edits an app and needs to read a shared library: add the
library under **Folders** in its pane, with **Can edit** off. The teammate
can read it without **Whole machine** access. See
[Teammates and drivers](/docs/setup/teammates/#folders).

## Talk it through

When typing is too slow, call the teammate with the phone key at the head of its conversation.
Describe the problem out loud; the teammate answers in a sentence or two and
writes the full answer, with code and links, into the chat. Its work keeps
going between your questions. See [Calls and dictation](/docs/use/voice/).

## Start fresh on a new subject

Finished one piece of work and starting another with the same teammate?
**More → Start a new chapter**. The teammate writes a handoff note and
starts the next subject with a clean context. If you went too far,
**Reopen previous chapter** takes it back one step. See
[Chapters](/docs/use/conversations/#chapters).
