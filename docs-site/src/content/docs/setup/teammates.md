---
title: Teammates and drivers
description: Hotline Agent, the harnesses Hotline can run as teammates, and what each teammate's pane controls.
---

Every teammate runs on one of two kinds of driver.

## Hotline Agent

Hotline's own agent, in-process, on a provider key you connected. It has
workspace file tools, a shell, Hotline's own tools (asking you for help,
messaging a teammate, scheduling itself), and any MCP servers you grant. Its
model and reasoning effort are picked from the title bar. Its shell and file
tools are confined to the workspace unless you grant **Whole machine**.

## A harness you already have

Any of these runs as a teammate in its own lane, with its own login, tools
and permission policy:

| Teammate | Needs on this machine |
| --- | --- |
| Claude Code | the `claude` CLI, and `npx` |
| Codex | the `codex` CLI, and `npx` |
| Cursor | `cursor-agent` |
| opencode | `opencode` |
| Gemini CLI | `gemini` |
| Grok Build | `grok` |

Hotline also reads the [Agent Client Protocol](https://agentclientprotocol.com)
registry of published agents once a day, so other agents that speak it show
up in the picker as they appear. A row the machine cannot start says what is
missing, usually a CLI that is not on your `PATH`. Hotline never downloads an
agent for you; `npx` fetches the small adapter that Claude Code and Codex
speak through.

The chat header for these teammates shows the model and effort the harness
advertises. Their own permission model shows in the pane as **Externally
managed**; Hotline passes on the modes the harness offers rather than inventing
its own. File reads and writes the harness asks Hotline to do stay inside the
workspace.

**Settings → General → New teammates run on** sets which driver a new
teammate gets.

## The teammate pane

Open a teammate and press <kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>I</kbd>. The
pane sits beside the conversation and controls:

- **Purpose.** The sentence written into the workspace as `AGENTS.md`.
- **Working directory.** Reveal it, or choose another folder.
- **Access.** Workspace only, or **Whole machine** (Hotline Agent).
- **Background work.** Whether the teammate may schedule its own wakes. Off
  by default. See [Schedules](/docs/setup/schedules/).
- **Collaboration.** Which other teammates it may message, granted per
  direction the first time it is tried.
- **Tools.** Which MCP servers it gets. See [Tools and MCP servers](/docs/setup/tools/).
- **Computer.** A containerized desktop of its own. See [The computer](/docs/setup/computer/).
- **Threads.** Its exchanges with other teammates.

Changing anything the driver is built from restarts the session. The
conversation keeps going from the same tape.
