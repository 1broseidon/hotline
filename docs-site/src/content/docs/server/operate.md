---
title: Day to day
description: Status, logs, restarts, updates, and what lives where.
---

## Check on it

```sh
hotline status
systemctl status hotline
journalctl -u hotline
```

`hotline status` asks the running desk for its version, uptime, room folder,
store, what it runs, teammates and connected models. A
[relay-only server](/docs/server/relay/) has no teammates or models, and
reports the address it answers on and how many devices are paired instead.
When the service is down it says so and exits with status 3.

## Stop and restart

`systemctl restart hotline` stops cleanly:

1. New work is refused with **Hotline is restarting. Try again in a moment.**
   A schedule that comes due meanwhile runs after the restart.
2. What you said that was still waiting behind a teammate's turn is kept for
   the next start.
3. Turns already running get up to 30 seconds to finish.
4. A turn still running after that is stopped, and the conversation says so.
   It is not run again, because it may already have done part of its work.

On the next start, the kept lines go to their teammates once, in order. A
desk killed outright (`kill -9`, power loss) keeps what was already written
and nothing more.

## One desk per room

A room has one desk at a time. A second `hotline serve`, the desktop app or
an import on the same room is refused while one runs.

## What lives where

Everything is under the room, `/var/lib/hotline/room`:

| Path | What |
| --- | --- |
| `room.jsonl`, `transcripts/` | The room and every conversation |
| `workspaces/` | Each teammate's own workspace |
| `secrets/` | Keys and pairings, one owner-only file each |
| `store.json` | Which secret store the room uses |
| `desk.lock` | The room's lock. Never delete it while the desk runs |
| `door.json` | How the terminal commands reach the running desk; removed on a clean stop |
| `serve.json` | What the server runs; absent means agents and relay |
| `pending.json` | What a restart kept for the next start; absent otherwise |

Back up the whole directory with the desk stopped, and encrypt the backup:
it holds your keys.

## Update

```sh
sudo hotline update
```

It installs the latest server release, restarts the desk and checks that it
comes back, and puts the old binary back if it doesn't. `hotline update --check`
only reports the installed and available versions. Paired phones stay paired.
See [Update a server](/docs/server/install/#update-a-server) for choosing a
release and what a rollback restores.
