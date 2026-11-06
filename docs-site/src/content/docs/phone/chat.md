---
title: Talk to your teammates
description: The team list, a conversation, and answering what a teammate is waiting on.
---

## The team list

Every teammate on your linked desks, with what each is doing. A teammate
waiting on you says **Needs you** and comes first. Search at the bottom finds
a teammate. The **+** beside it adds one: a name, an optional goal, and what
it runs on. The desk must be one that can add teammates from a phone.

## A conversation

Type and send. Replies stream in as the teammate writes them, and it keeps
working while the phone is closed.

- **+** beside the box attaches **Photos**, **Camera**, **Files**, or
  **Paste image** when there is an image to paste.
- Swipe a message left to reply to it.
- A wide table or a long piece of code is cut to fit, with a line saying
  what it is; tap it to open it.
- A file a teammate sends opens in place.

<!-- TODO(phone 0.13, story 7, task-66): check the open view's band, code colour and file names, the callout and diagrams against the build. -->

An opened table, code block, diagram or picture fills the screen under a
band with back, who said it and when, what it is ("Rust · src/lease.rs · 48
lines") and **Copy**, as on the desk. Code names its language and file and
colours its syntax, a `[!WARNING]` or `[!CAUTION]` quote is an amber
callout, and a diagram that branches is drawn.

<!-- TODO(phone 0.13, story 6, task-63): check where the colours come from and whether the phone can change them. -->

The phone wears the desk's colours: its **Accent** (**Settings → General**
on the desk) and each teammate's **Bubble**.

<!-- TODO(phone 0.13, story 9): check that the phone draws no card with the desk's switch off. -->

Link cards come through the desk, so **Link previews** off on the desk
(**Settings → General → Conversations**) turns them off on the phone too.

## Threads

<!-- TODO(phone 0.13, story 5, task-62): written from the plan. Check where Noted and Loop in… sit on the phone, and how a handed-off thread opens. -->

- **Noted** lists a teammate's asides: what its threads and colleagues told
  it without telling you, newest first.
- **Loop in…** in a work thread brings in a colleague, who answers once in
  the thread.
- A work thread one teammate handed to another opens like any thread, and
  you can talk in it.

See [Teammates working together](/docs/use/teammates-together/).

## When a teammate is waiting on you

A card in the conversation says what the teammate needs:

| Card | Your answers |
| --- | --- |
| Permission to use a tool | **Allow** or **Deny** |
| **Needs you**: something only you can do, like a sign-in | **Done** or **Won't do** |
| A passkey to create | **Approve** or **Deny** |
| A site to sign in to | **Use Cookies**, **Sign In**, **Open Browser**, or **Not now** |

The same buttons are on the notification. Your phone asks you to unlock it
first, then sends the answer.

<!-- TODO(phone 0.13, task-67): check the sign-in card against the build: the four answers, Face ID before Use Cookies, the browser choice, what Open Browser opens on the phone, and which buttons its notification carries. -->

### A sign-in

A teammate that needs to be signed in to a site asks with the site and its
reason, and counts as **Needs you** until you answer:

- **Use Cookies** shares that site's cookies from your browser on the desk.
  Choose the browser when several have it. The phone asks for Face ID first.
- **Sign In** takes a username and password, which go to the desk, never
  to the teammate.
- **Open Browser** opens the teammate's browser for you to sign in yourself.
- **Not now** declines; it won't ask for that site again until you next
  write to it.

See [Sign-ins](/docs/setup/sign-ins/) for what each answer does on the desk.
