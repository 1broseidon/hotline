---
title: Link with your desktop
description: Let the server's teammates and your desktop's message each other.
---

Adding a server to the desktop app lets *you* see and talk to the server's
teammates. **Linking** it is a separate step that lets the *teammates* see
and message each other: Clementine on your laptop can ask Lou on the server
to check the CI runners, the way she asks a teammate beside her.

## Link

On the desktop, with the server added: **Settings → Desks**, then
**Link teammates** on the server's row. The row then says **Teammates
linked**. The desktop keeps the link up by itself, through sleep and
restarts, and a message sent while the server is away waits and goes when it
is back.

From then on, each desk's teammates see the other desk's in their list of
teammates, with the desk's name beside them, and message them by name. If two
teammates share a name, a teammate says which it means, as **Lou on
grizzly**.

## What crosses, and what doesn't

Only each teammate's name and what it is doing, and the messages the
teammates send each other. Nothing about either desk's workspaces, tools,
keys, secrets or computers crosses a link.

The first time a teammate messages one on the other desk, the person is asked
on the *recipient's* desk, on that teammate's conversation, exactly as for a
teammate on the same desk. A teammate with **Whole machine** access on its own
desk is still asked about on the other.

The two teammates' thread is kept on both desks.

## See and end links

On the server:

```sh
hotline links
hotline unlink DESK_ID
```

Or **Unlink…** on the server's row in the desktop's **Settings → Desks**.
Ending a link ends it on both desks: the other desk's teammates leave the
list, a message still waiting fails with a sentence saying so, and the
grants given to them end too. Link again to start over.
