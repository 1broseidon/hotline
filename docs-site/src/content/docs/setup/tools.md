---
title: Tools and MCP servers
description: Add MCP servers to the room, and grant them to teammates.
---

Tools reach a teammate through the [Model Context Protocol](https://modelcontextprotocol.io).
Servers are added once, to the room, and granted per teammate.

## Add a server

**Settings → Tools → MCP servers**. A server is either:

- **Command**: a program Toad starts on this machine and speaks to over
  stdio. Give it the command, arguments and any **Environment** variables it
  needs.
- **Reached at a URL**: a server that speaks streamable HTTP. **Auth** is
  either **OAuth sign-in**, which Toad completes in your browser and keeps
  the tokens for, or a **Token** sent as a bearer header. Tokens are kept on
  this machine, bound to that server's URL.

Removing a server takes its tools away from every teammate on their next
start.

## Grant servers to a teammate

In the teammate's pane, **Which MCP servers this teammate gets**:

- **Default for new teammates**
- **Only the servers ticked below**
- **Every gateway server, including ones added later**

A grant takes effect on the teammate's next start. Teammates that run an
outside harness (Claude Code, Cursor and the rest) receive the same grants
through Toad, alongside whatever tools that harness configures itself.

## Toad's own tools

Every teammate, whichever driver, also has tools that belong to the room:

| Tool | What it does |
| --- | --- |
| `request_human` | Asks you for something only a person can do, and waits up to ten minutes. |
| `message_teammate` | Sends a message to another teammate, once you have allowed that pair. |
| `schedule`, `loop`, `list_schedules`, `cancel_schedule` | Wakes itself later, or on an interval, with **Background work** on. |

A teammate with a [computer](/docs/setup/computer/) gets that machine's tools as
one more server, whatever its MCP grant says.
