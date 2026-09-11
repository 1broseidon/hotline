---
title: Data and privacy
description: Where Toad keeps everything, and what leaves your machine.
---

Toad is local-first. There is no Toad account and no Toad server. What
leaves your machine is what a teammate sends to the model provider you
connected, what the harness you chose sends on its own behalf, and the
GitHub calls that check for updates and read the agent registry.

## The data directory

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/Toad` |
| Windows | `%APPDATA%\Toad` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/toad` |

Set `TOAD_DATA_DIR` to put it somewhere else. Inside it:

- the room: the roster, settings and schedules, as one append-only stream;
- one tape per teammate: its whole conversation, every chapter;
- threads between teammates;
- `workspaces/`, the folders of teammates you did not give a folder to;
- a full-text search index, rebuildable;
- `cache/`, the day's copy of the agent registry;
- `updater.json`, the last update check.

Back the directory up as a whole. Installing a new version of Toad never
touches it.

## The vault

`vault/` inside the data directory holds every secret: pasted API keys,
provider sign-in tokens, an HTTP MCP server's OAuth registration and tokens,
and the model lists discovered for each connection. Files are created
owner-only. Nothing in the vault is ever written into a conversation, a
teammate's workspace, an agent's instructions or a log.

Teammates that run an outside harness (Claude Code, Cursor and the rest)
keep their own logins where that tool keeps them; Toad holds nothing for them.

## Boundaries

- Toad Agent's file and shell tools are confined to the teammate's workspace
  unless you grant **Whole machine**.
- File reads and writes an outside harness asks Toad to perform stay inside
  the workspace, whatever mode the harness is in.
- A computer runs unprivileged in its container, with its one port on
  loopback and only the folders you mounted.
- One teammate messaging another is a grant you make, per pair, per direction.
