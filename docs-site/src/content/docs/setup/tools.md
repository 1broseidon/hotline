---
title: Tools, skills and web search
description: Add MCP servers and skills to the room, grant them to teammates, and set up web search.
---

A teammate's tools come from four places: the room's own tools, which every
teammate has; MCP servers and skills you add to the room and grant per
teammate; web search; and, for a teammate with one, its
[computer](/docs/setup/computer/).

## Hotline's own tools

Every teammate, whichever driver runs it, has these:

| Tool | What it does |
| --- | --- |
| `request_human` | Asks you for something only a person can do, as a **Needs you** card. It does not wait: your answer arrives later as a message. |
| `message_teammate`, `list_teammates` | Asks a colleague a question or hands it work, once you have allowed the pair, and sees who is idle, working or waiting. See [Teammates working together](/docs/use/teammates-together/). |
| `send_file` | Sends you a file in the conversation: a report, a chart, a screenshot of its computer. Pictures up to 2000 pixels, other files up to 25 MB. |
| `react` | Puts an emoji on your last message instead of a reply. |
| `open_link` | Opens a link in your own browser, only while you are at the desktop app. |
| `web_search` | Searches the web. See [Web search](#web-search). |
| `generate_image`, `set_avatar` | Draws a picture, and sets its own profile picture. See [Pictures](/docs/use/images/). |
| `search_thread`, `list_chapters`, `new_chapter`, `resume_chapter` | Searches its own history, including earlier chapters, side threads and calls, and opens or reopens a chapter. |
| `schedule`, `loop`, `list_schedules`, `cancel_schedule` | Wakes itself later, or on an interval, with **Background work** on. See [Schedules](/docs/setup/schedules/). |
| `computer_status` | Says whether its computer is ready or still downloading. |

Hotline Agent also has its own file tools and shell, confined to the
teammate's folders, and `subagent` to send work to a
[subagent](/docs/use/side-threads/#subagents). Coding tools such as Claude
Code bring their own.

## MCP servers

Tools from elsewhere reach a teammate through the
[Model Context Protocol](https://modelcontextprotocol.io). Servers are added
once, to the room, under **Settings → Tools → MCP servers**, and granted per
teammate. A server **Runs**:

- **On this computer**: a program Hotline starts and speaks to over stdio.
  Give it the command and any **Environment** variables it needs, one
  `NAME=value` per line, kept in the keychain.
- **At a URL**: a server that speaks streamable HTTP. **Signs in** is one of:
  - **No sign-in**;
  - **With an account**: OAuth. Hotline completes the sign-in in your
    browser and keeps the tokens;
  - **Token**: a token you paste, sent as a bearer token;
  - **Header**: a token you paste, sent in a header you name.

  Tokens are kept on this machine, bound to that server's address. On a desk
  on a server, the browser can't reach the server's own address: approve
  the sign-in, then paste the address of the page you land on (it won't
  load, and that is expected) and choose **Finish sign-in**.

Removing a server takes its tools away from every teammate on their next
start.

### Grant servers to a teammate

In the teammate's pane, under **Tools → MCP servers → Grant**:

- **None**: the default for new teammates.
- **Selected**: only the servers ticked below.
- **All**: every server in the room, including ones added later.

A grant takes effect on the teammate's next start. Coding tools such as
Claude Code receive the same grants through Hotline, alongside whatever
tools the tool configures for itself. Under the grant, the pane lists what
actually attached at the last start.

## Skills

A skill is a procedure a teammate reads when a task calls for it: how to
cut a release, how to work a computer, the thing you asked for twice last
week. It is a folder holding a `SKILL.md`, in the
Agent Skills format other agents read too.

**Settings → Skills** has three groups:

- **Your skills**: the skills in `~/.agents/skills`, the folder other agents
  on this machine read, or a folder you choose with **Change**. Switch each
  one on to offer it to teammates, or **Offer all**. Edits there reach a
  teammate at its next start.
- **From other folders**: a skill from anywhere else. **Add a skill folder**
  copies it into Hotline's data directory.
- **Built in**: Hotline's own procedures, which every teammate has.

Grant what is offered in the teammate's pane, under **Tools → Skills**, with
the same **None**, **Selected** and **All** as servers. Hotline copies the
granted skills into the teammate's working directory under
`.agents/skills/`, so every driver can read them, and removes its copies when
a grant is taken away. Skills you or the teammate wrote there yourselves are
never touched.

## Web search

Every teammate can search the web with `web_search`, with no setup. Each
search asks several search providers at once (Parallel, Exa, Keenable and
Firecrawl) and merges their answers, so one weak provider does not decide
the result. Only the query is sent.

**Settings → Tools → Web search** lists the providers. Switch one off to
stop every teammate using it. **Add key** gives a provider your own API key,
kept in the keychain: a provider with a key counts for more and lifts its
free rate limit.
