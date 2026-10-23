---
title: Data and privacy
description: Where Hotline keeps everything, and what leaves your machine.
---

Hotline is local-first. There is no Hotline account and no Hotline cloud.
Your room lives on your desk: this computer, or a server of your own.

## What leaves your machine

- What a teammate sends to the AI service it runs on, and what a coding tool
  such as Claude Code sends on its own behalf.
- Speech and pictures sent to the provider chosen under **Use for**, when it
  is not a model on this machine.
- Web search queries, to the search providers switched on under
  **Settings → Tools → Web search**. Only the query is sent.
- Whatever a teammate's tools, MCP servers and computer reach on the network.
- The link previews shown under a message.
- Update checks to GitHub, for Hotline and for the computer image, and the
  daily read of the [Agent Client Protocol](https://agentclientprotocol.com)
  registry.
- Notifications to a paired phone. Your phone talks to your desk over an
  encrypted channel; a relay or tunnel in between cannot read it.

## The data directory

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Hotline` |
| Windows | `%APPDATA%\Hotline` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/hotline` |

Set `HOTLINE_DATA_DIR` to put it somewhere else. Inside it:

- `room.jsonl`: the roster, settings and schedules, as one append-only
  record;
- `transcripts/`: one per teammate, its whole conversation, every chapter;
- `threads/`, `sides/`, `runs/` and `calls/`: conversations between
  teammates, side threads and work threads, subagent runs, and calls;
- `files/` and `avatars/`: Hotline's copies of files sent in a conversation,
  and teammates' pictures;
- `workspaces/`: the folders of teammates you did not give a folder to;
- `skills/`: skills added under **From other folders**;
- `acp-agents/`: agents downloaded from the registry;
- `agent-accounts/`: one folder per [Claude subscription](/docs/setup/providers/#claude-subscriptions),
  where Claude Code keeps that account's sign-in, settings and history;
- `index.sqlite`: the search index, which can be rebuilt;
- `cache/`: the day's copy of the agent registry, and the desk's speech
  models;
- `logs/hotline.log`: the desktop app's own log.

Back the directory up as a whole. Installing a new version of Hotline never
touches it.

## The log

When something goes wrong, such as a teammate that never starts or an app that
quit unexpectedly, look in `logs/hotline.log` in the data directory and attach
it when you report the problem. It holds what the desktop app noticed while
starting up, why an agent failed to start, and any crash message. Hotline writes
no keys or tokens to it, and it is not the conversation; that stays in the
transcripts. It is capped at 5 MB, with the previous file kept as
`hotline.log.1`. A server desk logs to its service's journal instead
(`journalctl -u hotline`).

## Keys and secrets

API keys, sign-ins to OpenRouter and xAI, MCP server credentials, web search
keys and everything under **Settings → Secrets** are kept in the operating
system's credential store: the macOS Keychain, Windows Credential Manager, or
the Secret Service on Linux. The data directory holds only references to
them. ChatGPT and GitHub Copilot sign-ins are kept in owner-only files in the
data directory. A desk on a server, which has no keychain, keeps its secrets
in owner-only files under its room.

Nothing secret is ever written into a conversation, a teammate's folder, an
agent's instructions or a log.

Teammates that run a coding tool such as Claude Code keep their own logins
where that tool keeps them, or in a Claude subscription's folder; Hotline
holds nothing for them.

## Boundaries

- Hotline Agent's file and shell tools are confined to the teammate's
  folders unless you grant **Whole machine**.
- File reads and writes a coding tool asks Hotline to perform stay inside the
  teammate's folders, whatever mode the tool is in. The tool's own shell
  follows the tool's own permissions.
- A computer runs unprivileged in its container, with its one port on
  loopback and only the folders you gave it.
- One teammate reaching another is a grant you make, per pair, per
  direction. A Hotline Agent teammate with **Whole machine** needs no grant.
- A phone can't widen what a teammate reaches. Folders, tools, MCP servers,
  computers, providers and keys change only at the desk.
