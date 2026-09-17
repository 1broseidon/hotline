---
title: How a room works
description: Teammates, conversations, chapters, threads and what the person is asked.
---

## Teammates

A teammate is a named agent with its own working directory, its own
conversation, and its own settings. Nine seats are one keystroke away
(<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>1</kbd>…<kbd>9</kbd>). Removing a teammate
removes its conversation too; Hotline asks first.

## Conversations and chapters

A conversation is one long tape. Hotline closes a **chapter** after the
teammate has been idle for a while (**Settings → General → Close a chapter
after**, in hours; the default is 8) and opens a fresh one on the next message, carrying a
short note of where things stood. Search (<kbd>⌘</kbd>/<kbd>Ctrl</kbd> +
<kbd>F</kbd>) covers every chapter.

## Access

Each teammate works inside its **workspace**, the folder you gave it. Hotline's
own file and shell tools are confined to it. **Whole machine** access, in the
teammate's pane, lifts that wall for Hotline Agent. Teammates that run an outside
harness bring that harness's own permission model; Hotline shows it as
**Externally managed**.

## When a teammate needs you

A teammate can ask for something only a person can do: a login, a tap on a
phone, a CAPTCHA, a decision. A **Needs you** card appears in the conversation and the teammate waits
until you mark it done or decline it. If it has a computer, the card opens
that desktop so you can do it there. Nobody answering for ten minutes counts
as a decline.

Teammates that run an outside harness may also ask permission before an
action; that arrives as a card in the same place.

## Teammates talking to each other

A teammate can message another teammate. Each direction is granted once by
you, the first time it is tried, in the **Collaboration** section of the
teammate's pane. The exchange shows up as a thread on both conversations.
