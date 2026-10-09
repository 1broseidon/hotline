---
title: Data and privacy
description: Where Hotline keeps everything, and what leaves your machine.
---

Hotline is local-first. There is no Hotline account and no Hotline server. What
leaves your machine is what a teammate sends to the model provider you
connected, what the harness you chose sends on its own behalf, and the
GitHub calls that check for updates and read the agent registry.

## The data directory

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Hotline` |
| Windows | `%APPDATA%\Hotline` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/hotline` |

Set `HOTLINE_DATA_DIR` to put it somewhere else. Inside it:

- the room: the roster, settings and schedules, as one append-only stream;
- one tape per teammate: its whole conversation, every chapter;
- threads between teammates;
- `workspaces/`, the folders of teammates you did not give a folder to;
- a full-text search index, rebuildable;
- `cache/`, the day's copy of the agent registry;
- `logs/hotline.log`, the desktop app's own log;
- `updater.json`, the last update check.

Back the directory up as a whole. Installing a new version of Hotline never
touches it.

## The log

When something goes wrong, such as a teammate that never starts or an app that
quit unexpectedly, look in `logs/hotline.log` in the data directory and attach
it when you report the problem. It holds what the desktop app noticed while
starting up, why an agent failed to start, and any crash message. Hotline writes
no keys or tokens to it, and it is not the conversation; that stays in the tapes. It
is capped at 5 MB, with the previous file kept as `hotline.log.1`. A server desk
logs to its service's journal instead (`journalctl -u hotline`).

## The vault

`vault/` inside the data directory holds every secret: pasted API keys,
provider sign-in tokens, an HTTP MCP server's OAuth registration and tokens,
and the model lists discovered for each connection. Files are created
owner-only. Nothing in the vault is ever written into a conversation, a
teammate's workspace, an agent's instructions or a log.

Teammates that run an outside harness (Claude Code, Cursor and the rest)
keep their own logins where that tool keeps them; Hotline holds nothing for them.

## Boundaries

- Hotline Agent's file and shell tools are confined to the teammate's workspace
  unless you grant **Whole machine**.
- File reads and writes an outside harness asks Hotline to perform stay inside
  the workspace, whatever mode the harness is in.
- A computer runs unprivileged in its container, with its one port on
  loopback and only the folders you mounted.
- One teammate messaging another is a grant you make, per pair, per direction.
