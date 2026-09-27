# Hotline on a server

`hotline serve` runs the desk with no window: the same room, teammates,
scheduler and Remote access as the desktop app, on a Linux machine you
reach over SSH. It needs no display and no session bus. This page covers
what ships today; pairing a phone to it (BRO-114) and setting it up from the
phone (BRO-116) are the next steps of BRO-16.

## Install

The release has `hotline-server_<version>_linux_x86_64.tar.gz` and
`_linux_aarch64.tar.gz`, holding the `hotline` binary and a systemd unit.

```sh
tar xzf hotline-server_*_linux_x86_64.tar.gz
sudo install -m 0755 hotline-server_*/hotline /usr/local/bin/hotline
sudo useradd --system --create-home --home-dir /var/lib/hotline --shell /usr/sbin/nologin hotline
sudo install -m 0644 hotline-server_*/hotline.service /etc/systemd/system/hotline.service
sudo systemctl daemon-reload
sudo systemctl enable --now hotline
```

The unit runs `hotline serve --store file` as `hotline`, with the room at
`/var/lib/hotline/room`. It sets `HOME`, `PATH` and `HOTLINE_DATA_DIR`
itself instead of reading a login shell, so add to its `PATH` whatever
your teammates' tools need (Node for `npx`, a harness's install directory).
Docker or Podman is needed only for teammates with a computer.

## Check on it

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline status
systemctl status hotline
journalctl -u hotline
```

`hotline status` asks the running desk, through its loopback Door, for its
version, uptime, store, teammates and connected models. When the service is
down it says so and exits 3.

`hotline wire <command>` sends one wire command (`docs/wire.md`) on the
desk seat and prints the result. It reads the params as JSON on stdin,
never from the command line, so a key in them stays out of shell history
and the process list:

```sh
echo '{}' | sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline wire providers.list
```

It is an escape hatch for what the phone cannot do yet, not the way to run
the desk.

## The secret store

`--store` is required. `file` keeps secrets as owner-only files under
`<data>/secrets/`; `native` uses the OS credential store, which on a server
usually is not there. The room records the choice in `store.json` and
refuses a desk started on the other one, rather than opening on an empty
store beside keys it cannot reach. Moving a room between stores means
entering its keys and pairing its phones again.

The files are protected by the `hotline` account and the disk, nothing
more: encrypt the volume, and its snapshots and backups
(`docs/security.md`, "A served desk's secrets are files").

## One desk per room

`serve` takes the room's lock before it opens anything. A second `serve`,
the desktop app, or `hotline-import` on the same data directory is refused
while it runs. The lock goes with the process, however it ends.

## Stopping

On SIGTERM (`systemctl stop`, `restart`) the desk stops for a restart:

1. New work is refused with "Hotline is restarting. Try again in a moment."
   A schedule that comes due meanwhile is retried after the restart.
2. What the person said that is still waiting behind a teammate's turn is
   written to `pending.json` straight away.
3. Turns already running get up to 30 seconds to finish.
4. A turn still running after that is stopped, and the teammate's
   conversation says so. It is not run again: it may already have done part
   of its work, and doing that twice is worse than asking.
5. Every stream is synced, and the process exits.

On the next start, each line in `pending.json` is handed to its teammate
once, in order, and the file is removed. A line the teammate has read
meanwhile is skipped; a scheduled line whose teammate lost background work
is dropped. None of this is exactly-once. A desk killed outright
(`SIGKILL`, power loss) keeps what was already written and nothing more.

## Files

| Path | What |
| --- | --- |
| `desk.lock` | The room's lock. Never delete it while a desk runs. |
| `door.json` | The Door's loopback port and this process's token, 0600, removed on a clean stop. |
| `store.json` | Which secret store the room uses. |
| `secrets/` | The file store, 0700, one 0600 file per record. |
| `pending.json` | Lines kept by the last stop for the next start; absent otherwise. |
