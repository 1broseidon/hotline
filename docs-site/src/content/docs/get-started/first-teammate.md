---
title: Your first teammate
description: The welcome, connecting an AI service, naming a teammate and saying hello.
---

A new Hotline opens on **Welcome to Hotline**. **Get started** walks you
through setup in four short steps. If your teammates run on a server you
run, choose **Connect to a server instead**, then run
`hotline pair --link` on the server and paste what it prints, or pair over
SSH. See [Hotline on a server](/docs/server/).

## 1. How your teammates think

A teammate needs an AI to think with. There are two kinds:

- **Hotline Agent**, Hotline's own agent, on an AI service you connect:
  ChatGPT, Claude, Grok, Gemini, a model on your own machine, and more. This
  is the recommended choice if you are not sure.
- **An AI coding tool you already have**: Claude Code, Codex, Cursor, Gemini
  CLI, Grok Build and others that run on this computer. They use your own
  account with that tool, so there is nothing to connect in Hotline.

You can pick differently for any teammate later.

## 2. Connect

For Hotline Agent, connect an AI service. The welcome shows six popular ones
(Anthropic, ChatGPT, OpenRouter, xAI, Ollama and Copilot), with the rest a
click away. Sign in with an account you already have, or paste an API key.
Keys are kept in your computer's keychain, never in a conversation or a
file a teammate can read. One service is enough to start. The full list is
in [Providers and keys](/docs/setup/providers/).

For a coding tool, pick it. The first time a teammate uses it, the tool may
ask you to sign in, in its own way.

## 3. Meet your first teammate

Give it a **Name** (the die suggests one) and a **Goal**: what it should
help with, in your own words. The goal is written into its folder as
`AGENTS.md`, so the agent reads it every time it starts. **Start from**
offers goals for code, research and writing to adapt.

Everything else starts with a good default, and you can change it later in
the teammate's pane. Later teammates are added with
<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>N</kbd> or the **+** at the foot of the
team list. That form also has:

- **Thinks with**: which agent runs it, and, for Hotline Agent, which model.
- **Folders**: its working directory. Leave it blank and Hotline makes a
  folder of its own under the data directory, or pick a project of yours.
  **Also let it read** adds further folders. See
  [Folders](/docs/setup/teammates/#folders).
- **Abilities**: **Whole machine**, **Background work** and **Computer**.
- **Picture**: when a provider that draws is connected, Hotline can draw
  the teammate a face from its name and goal.

## 4. Say something

The teammate takes a seat in the team list. Type in the message box and
press <kbd>Enter</kbd> (<kbd>Shift</kbd> + <kbd>Enter</kbd> for a new line).
The teammate starts, reads its folder, and works. A card under the header
follows its steps, and the answer lands in the conversation.
<kbd>Esc</kbd> interrupts a turn.

You can leave. The conversation carries on, and a toast tells you when the
teammate finishes or needs you.

## Where next

- See what to use it for: [Common workflows](/docs/use/workflows/).
- Understand the room: [How a room works](/docs/get-started/room/).
- Give it tools: [Tools, skills and web search](/docs/setup/tools/).
- Give it a desktop: [The computer](/docs/setup/computer/).
- Have it wake itself: [Schedules](/docs/setup/schedules/).
