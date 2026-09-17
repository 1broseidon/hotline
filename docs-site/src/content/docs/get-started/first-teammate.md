---
title: Your first teammate
description: Add a provider, name a teammate, give it a folder, say hello.
---

Hotline opens on an empty room. Three steps put a teammate in it.

## 1. Connect a provider

Hotline's built-in agent, **Hotline Agent**, runs on a model key you hold. Open
**Settings** (<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>,</kbd>), pick **Providers**,
and choose **Add provider**. Paste an API key, or sign in where the provider
offers it. The key is stored in Hotline's vault on this machine and nowhere
else. The full list is in [Providers and keys](/docs/setup/providers/).

If you would rather drive a tool you already have, such as Claude Code or
Cursor, skip this step: those teammates sign themselves in. See
[Teammates and drivers](/docs/setup/teammates/).

## 2. New teammate

<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>N</kbd>, or the plus at the top of the
rail. The form asks for:

- **A name.** Teammates are people in the room; you will address them by it.
- **What this teammate is for.** A sentence or two. It is written into the
  working directory as `AGENTS.md`, so the agent reads it on every start.
- **A working directory.** Leave it blank and Hotline makes a folder under its
  data directory; or pick a project of your own.
- **Which agent runs it.** Hotline Agent, or any harness this machine can start.
- **A model**, for Hotline Agent.

## 3. Say something

The teammate takes a seat in the rail. Type in the composer and press
<kbd>Enter</kbd> (<kbd>Shift</kbd> + <kbd>Enter</kbd> for a new line). The
teammate starts, reads its folder, and works. Steps fold up as it goes, and
the answer lands in the conversation. <kbd>Esc</kbd> interrupts a turn.

You can leave. The conversation carries on, and a toast tells you when the
teammate finishes or needs you.

## Where next

- Give it tools: [Tools and MCP servers](/docs/setup/tools/).
- Give it a desktop: [The computer](/docs/setup/computer/).
- Have it wake itself: [Schedules](/docs/setup/schedules/).
- Understand the room: [How a room works](/docs/get-started/room/).
