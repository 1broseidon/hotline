---
title: Working in a conversation
description: Sending, steering and stopping a teammate, replies and reactions, attachments, search and chapters.
---

Each teammate has one conversation that goes on for as long as the
teammate exists. This page covers what you can do in it.

## Send, steer, stop

Type in the message box and press <kbd>Enter</kbd>
(<kbd>Shift</kbd> + <kbd>Enter</kbd> starts a new line).

While the teammate is working, a card floating under the header shows the
steps of the turn as they happen: the commands it runs, the files it reads,
the edits it makes, and a **Thinking** row you can open. The teammate's
mark at the foot of the conversation shows what kind of work it is doing.

You do not have to wait for it to finish:

- **Send another message** and it reaches the teammate while it works. On
  Hotline Agent, Claude Code and Codex it joins the task already in hand;
  other agents read it when they finish the turn.
- **Interrupt** the turn with <kbd>Esc</kbd> in the message box.
  **More → Stop the session** stops the agent altogether; your next message
  starts it again.

When a turn fails at the network, at the provider or on a rate limit, its
error card offers **Try again**. That runs the same request again without
repeating your message.

## Replies, reactions and copying

Hover over a message for a small bar with **React**, **Reply** and **Copy**,
or right-click it for the same actions as a menu. With a few words selected,
right-click copies just those.

- **Reply** quotes the message above the box ("Replying to…"). Press
  <kbd>R</kbd> on a teammate's message to do the same from the keyboard, and
  <kbd>Esc</kbd> to stop replying. Clicking a quote in the conversation goes
  back to the message it quotes.
- **React** offers six emoji, the same ones as on the phone.

Code blocks have a copy key in their corner. A bare link gets a preview
card. Mermaid diagrams are drawn in place, with a toggle to see their source.
Pictures and diagrams open full size in the window's own viewer.

## Attach files and pictures

The **+** beside the message box (**Attach a file**) adds files. You can also
drop them on the window or paste them. A pasted screenshot becomes an
attachment named "Pasted image" and the time. See
[Pictures](/docs/use/images/) for what a teammate can see in an image.

A teammate can send you files back with `send_file`: a report, a chart, a
screenshot of its computer. A picture shows in the conversation; a PDF opens
when you press **Open**; any file can be saved where you choose. Nothing
opens by itself.

## Search

<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>F</kbd>, or the magnifier in the title
bar, searches the open teammate's conversation, every chapter of it
(**Here**), or every teammate's (**Everywhere**). With nothing typed, the
search lists the chapters themselves, each with its handoff note.

## Chapters

A model can only hold so much at once, so a conversation is divided into
**chapters**. Each chapter is one working context for the agent. You see a
chapter as a divider across the conversation with a title and a time.

A chapter closes in one of three ways:

- **After a quiet stretch.** **Settings → General → Conversations → Start a
  new chapter after** sets how many quiet hours that takes. The default is
  8. Quiet time counts from the last message, and a turn still running is
  never cut off.
- **When you ask.** **More → Start a new chapter** closes the current one
  now. Do this when you change subject, so the teammate starts clean.
- **When the teammate asks.** A teammate can close its own chapter when it
  finishes a piece of work.

When a chapter closes, a model writes a **handoff note**: the title, the
goal, the outcome, what is still open, the decisions made and the files
touched. The next chapter starts from that note and the last few lines, not
from the whole history. The teammate can still search its earlier chapters
when it needs something from them.

**More → Reopen previous chapter** goes back one chapter: the teammate picks up the
context it had there. You can go back only one step, and only to a chapter
that ran on the same agent; when you can't, the menu item says why.

A long piece of work stays in one chapter. When a Hotline Agent teammate
fills its model's context in the middle of a turn, it makes room and
carries on in the same chapter. Claude Code, Codex and the other harnesses
manage their own context.

## More

The **More** menu in the conversation's header also has:

- **Start a side thread** and **Threads**: see
  [Side threads and subagents](/docs/use/side-threads/).
- **Reveal working directory**, which opens the teammate's folder.
- **Show teammate**, which opens its pane
  (<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>I</kbd>, or press the teammate's name).
- **Remove teammate…**, which deletes the teammate and its conversation
  after asking.
