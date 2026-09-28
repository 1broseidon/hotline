# Hotline on a server

`hotline serve` runs the desk with no window: the same room, teammates,
scheduler and Remote access as the desktop app, on a Linux machine you
reach over SSH. It needs no display and no session bus. This page covers
the sealed phone connection. Owner onboarding from the phone (BRO-116)
is separate; until then use the local Door for setup.

## Install

The release has `hotline-server_<version>_linux_x86_64.tar.gz` and
`_linux_aarch64.tar.gz`, holding the `hotline` binary and a systemd unit.

```sh
tar xzf hotline-server_*_linux_x86_64.tar.gz
sudo install -m 0755 hotline-server_*/hotline /usr/local/bin/hotline
sudo useradd --system --create-home --home-dir /var/lib/hotline --shell /usr/sbin/nologin hotline
sudo install -m 0644 hotline-server_*/hotline.service /etc/systemd/system/hotline.service
# Edit the unit's --listen and --public-url for this host before starting.
sudo systemctl edit --full hotline
sudo systemctl daemon-reload
sudo systemctl enable --now hotline
```

The unit runs `hotline serve --store file` with explicit network flags as
`hotline`, with the room at
`/var/lib/hotline/room`. It sets `HOME`, `PATH` and `HOTLINE_DATA_DIR`
itself instead of reading a login shell, so add to its `PATH` whatever
your teammates' tools need (Node for `npx`, a harness's install directory).
Docker or Podman is needed only for teammates with a computer.

## Listen and TLS

Choose exactly one local IP and a fixed port. The public URL is where the
phone connects, and may be a DNS name or a TLS-terminating proxy:

```sh
hotline serve --store file --listen 192.0.2.10:9443 \
  --public-url https://desk.example:9443 --tls self
```

Replace the example address with this host's actual address. Wildcards,
port zero and fallback to a different address are refused. If the address
is late at boot, Remote waits for it without widening; the local Door stays
available for status. `--listen` and `--public-url` are required. IPv6 uses
brackets (`--listen '[2001:db8::10]:9443'`). For a supplied certificate use
`--tls-cert /path/fullchain.pem --tls-key /path/key.pem` instead of
`--tls self`; both PEM files must be readable by the service account.

The remote endpoint still uses TLS, but phone identity trust is the desk's
persistent X25519 Noise key, not the TLS certificate. Certificates can
rotate without revoking sealed pairings. A proxy terminates TLS and
forwards WebSockets to the desk's TLS listener; it only sees Noise
ciphertext for the handshake payloads and application frames. It can
still observe connection timing and sizes and deny service. Do not put
credentials in `--public-url` or its query string.

## Pair a device

As the service account, on the machine running the desk:

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline pair
# Or grant the existing limited phone seat explicitly:
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline pair --companion
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline devices
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline revoke DEVICE_ID
```

Scan the terminal QR with Hotline on the phone. The two-minute invitation
is single-use; the command waits and prints the device name and role.
Treat the QR and terminal scrollback as sensitive until it expires. Cancel
with Ctrl-C. A served desk does not offer the six-digit manual pairing
route, and its v2 claim endpoint is absent when no pairing window is open.
Removing every device does not open an owner bootstrap route: run `pair`
again explicitly. An owner has the same commands and subscriptions as the
local desk, including providers, grants and server paths. A companion keeps
the limited phone command set. Existing grants without a role remain owners.

The default command prints a pasteable link beside the QR. `hotline pair --json`
prints one JSON payload to stdout; `hotline pair --link` prints only the link.
Both wait for a claim, cancellation or expiry, so an SSH caller must keep the
process alive after reading the first line. Completion and errors use stderr.
The link is `hotline://pair?p=<base64url(JSON)>`, without base64 padding. Its
payload fields are `version` (2), `url` (HTTPS), `deskKey` (base64url X25519),
`secret`, `role`, `expiresAt` (Unix milliseconds), and `name`. The existing
phone QR query keeps `v`, `u`, `k`, `s` and adds `r`, `e`, `n` for the same
role, expiry and name. All forms name the same single-use invitation.

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
