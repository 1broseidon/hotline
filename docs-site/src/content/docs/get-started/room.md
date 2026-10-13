---
title: How a room works
description: Teammates, conversations, chapters, threads and what you are asked.
---

A **room** is everything one Hotline holds: the teammates, their
conversations, their schedules and the settings. It lives on one machine,
the **desk**: the desktop app on your computer, or `hotline serve` on a
server. A window or a phone is a way in to a desk.

## Teammates

A teammate is a named agent with a goal, its own folder, its own
conversation and its own settings. The team list at the left shows each
one, with what it is doing. Its mark moves while it works and rings when it
is waiting on you.

- <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>1</kbd>…<kbd>9</kbd> opens the first
  nine.
- **Pin to top**, at the foot of a teammate's pane or in its right-click
  menu, keeps up to three teammates above the list as larger faces, on every
  device.
- Drag the list's edge to resize it. Drag it narrow and it becomes a column
  of faces; <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>B</kbd> hides and shows it.

Removing a teammate removes its conversation too; Hotline asks first.

## Conversations and chapters

A teammate's conversation is one long record that never resets. Hotline
divides it into **chapters**, each one working context for the agent. A
chapter closes after a quiet stretch (8 hours by default), when you start a
new one, or when the teammate does. The next chapter starts from a short
handoff note of where things stood. Search
(<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>F</kbd>) covers every chapter. See
[Working in a conversation](/docs/use/conversations/#chapters).

Beside the main conversation, a teammate can have **threads**: side threads
you start, work handed to it by a colleague, its conversations with other
teammates, the subagents it runs, and calls. They open in the Threads pane
on the right. See [Side threads and subagents](/docs/use/side-threads/).

## Reach

Each teammate works inside its **folders**: its own working directory, and
any others you add. Hotline Agent's file and shell tools are confined to
them. **Whole machine**, under **Permissions** in the teammate's pane, lifts
that wall for Hotline Agent.

Teammates that run an outside harness, such as Claude Code, bring that
harness's own permission model. The pane says **Managed by** the harness and
offers the modes the harness advertises. File reads and writes the harness
asks Hotline to do stay inside the teammate's folders.

## When a teammate needs you

A teammate can ask for something only a person can do: a login, a tap on a
phone, a CAPTCHA, a decision. A **Needs you** card appears in the
conversation, and the teammate's face in the list shows it is waiting on
you. Choose **Done** or **Decline**, with **Add a note** if you have
something to say. If the teammate has a computer, **Open the screen** takes
you to its desktop to do it there.

The teammate does not sit and wait. It carries on with what it can, and
your answer arrives as a new message that wakes it if it was idle. A card
nobody answers in a day expires.

Teammates that run an outside harness may also ask permission before an
action. That arrives as a card in the same place, with the choices the
harness offers.

## Teammates talking to each other

A teammate can ask another a question, or hand it a piece of work. You
approve each direction the first time it is tried. See
[Teammates working together](/docs/use/teammates-together/).
