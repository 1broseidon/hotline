---
title: Teammates and drivers
description: Hotline Agent, the coding tools Hotline can run as teammates, and what each teammate's pane controls.
---

Every teammate runs on one of two kinds of driver.

## Hotline Agent

Hotline's own agent, running inside Hotline on an AI service you
[connected](/docs/setup/providers/). It has file tools and a shell confined
to the teammate's folders, Hotline's own tools (asking you for help,
messaging a colleague, scheduling itself, searching the web, drawing), any
MCP servers you grant, and subagents it can send off. Pick its model and
reasoning effort beside its name at the head of the conversation.

## A coding tool you already have

Any of these runs as a teammate, with its own login, tools and permission
modes:

| Teammate | Needs on this machine |
| --- | --- |
| Claude Code | the `claude` CLI, and `npx` |
| Codex | the `codex` CLI, and `npx` |
| Cursor | `cursor-agent` |
| opencode | `opencode` |
| Gemini CLI | `gemini` |
| Grok Build | `grok` |
| Muse Code | the `muse` CLI, and `npx` |

Hotline also reads the [Agent Client Protocol](https://agentclientprotocol.com)
registry of published agents once a day, so other agents that speak it show
up in the picker as they appear. A row the machine cannot start says what is
missing, usually a CLI that is not installed, or installed for another
account or outside the `PATH` Hotline sees.

Hotline does not install the CLIs above for you. `npx` fetches the small
adapter that Claude Code, Codex and Muse Code speak through. An agent the
registry publishes as a ready-made download is fetched the first time it
starts, checked against its published checksum when there is one, and kept
in the data directory.

Claude Code signs in with its own account, through Claude Code itself.
Hotline runs the `claude` CLI you installed and never asks for or relays your
Claude login. To give Claude Code teammates different Claude logins, see
[Claude subscriptions](/docs/setup/providers/#claude-subscriptions).

The conversation's header shows the model and effort the tool advertises.
In the teammate's pane, **Permissions** says **Managed by** the tool and
offers the modes it advertises; Hotline passes them on rather than inventing
its own. File reads and writes the tool asks Hotline to do stay inside the
teammate's folders. A tool's own shell is not confined by Hotline: choosing
one trusts its permission model.

A message you send while a Claude Code, Codex or Muse Code teammate is
working reaches the task it is on. Other tools read it when they finish the
turn.

**Settings → General → New teammates → Thinks with** sets which driver a
new teammate gets, and **Model** which model, when that is Hotline Agent.

## The teammate pane

Open it with <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>I</kbd>, or by pressing the
teammate's name at the head of the conversation. From the top:

- **The card**: its picture, **Name** and **Goal**, each edited in place,
  and its **Voice** on [calls](/docs/use/voice/). The goal is written into
  its working directory as `AGENTS.md`.
- **Folders**: see below.
- **Permissions**: **Whole machine** (Hotline Agent) or the tool's own
  modes; **Background work**, whether it may schedule its own wakes; and
  **Collaboration**, which colleagues may hand it work, each with **Revoke**.
- **Computer**: a desktop of its own. See [The computer](/docs/setup/computer/).
- **Tools**: which MCP servers and skills it gets, and what attached at its
  last start. See [Tools, skills and web search](/docs/setup/tools/).
- **Schedules**: see [Schedules](/docs/setup/schedules/).
- **Threads**: its conversations with other teammates.
- At the foot, **Remove teammate…** and **Pin to top**.

Changing anything a teammate's session is built from (its folders, reach,
tools, background work, goal or driver) restarts the session. The
conversation keeps going from where it was.

## Folders

A teammate's own folder is its working directory, shown as **Main
workspace** when Hotline made it. **Choose another folder** moves it to a
project of yours.

**Add folder** gives it more: a repository it should read, or a second
project it may edit, without **Whole machine**. Each extra folder is
read-only unless you turn on **Can edit**.

- Hotline Agent's file tools and shell open each one read-only, or
  read-write with **Can edit**, and nothing outside them.
- A coding tool is passed the folders when it accepts extra folders, and
  keeps its own permissions over them.
- A teammate's computer sees them under `/home/agent/folders`.

Your home folder, `/`, Hotline's own data and the teammate's own folder
cannot be added.
