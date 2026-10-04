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
`hotline` (or the user the installer was told to use, below), with the room at
`/var/lib/hotline/room`. It sets `HOME`, `PATH` and `HOTLINE_DATA_DIR`
itself instead of reading a login shell, so add to its `PATH` whatever
your teammates' tools need (Node for `npx`, a harness's install directory).
Docker or Podman is needed only for teammates with a computer.

## Harnesses, and the account the desk runs as

The desk finds a harness's CLI (`claude`, `codex`, `gemini` and the rest) on
its own `PATH`, as its own user, and a harness signs in with files in that
user's home (`~/.claude.json`, `~/.codex/auth.json`). The unit runs the desk
as `hotline`, with `HOME=/var/lib/hotline`, so a CLI you installed and signed
in to as yourself is invisible to it: the teammate list shows the row as not
available.

The row says why. When the CLI is installed for another user on this machine:

> claude is installed for agent, but the desk runs as hotline. Run the desk as
> agent (see docs/serve.md), or install claude for hotline.

When it is in the desk's own home or a system folder but not on its `PATH`:

> claude is at /var/lib/hotline/.local/bin/claude, which is not on the desk's
> PATH. Add /var/lib/hotline/.local/bin to PATH in the service unit (see
> docs/serve.md).

Anything else stays "Not installed". The desk only checks that an executable
exists, in `~/.local/bin`, `~/.npm-global/bin` and `~/.claude/local` of each
login account, and never opens anyone's files; a home it may not enter, which
Ubuntu makes the default, says nothing.

Two ways to fix it:

- **Run the desk as the user who has the CLIs.** On a first install, the
  installer does this for whoever ran it, as themselves or with `sudo`. Run
  as root with nobody behind it, as on a fresh VPS, it uses the one person's
  account on the machine if there is exactly one, and the service account
  otherwise; the desk never runs as root. `--user NAME` names someone else
  (`--user hotline` asks for the separate service account):

  ```sh
  curl -fsSL https://hotline.dev/install | sh -s -- --server --user "$USER"
  ```

  It sets `User=`, `Group=`, `HOME` and a `PATH` that starts with
  `~/.local/bin` and `~/.npm-global/bin`, creates `/var/lib/hotline` open to
  enter, and makes the room, `/var/lib/hotline/room`, that user's alone. It
  does not create the `hotline` account. The installer only chooses at the
  first install: `--user` on an installed desk is refused, and an upgrade
  never changes the user.
- **Keep the service account and install the CLI for it,** signing in as that
  account. Its login shell is `nologin`, so open one with
  `sudo -u hotline -H bash`; the unit's `PATH` already starts with its
  `~/.local/bin` and `~/.npm-global/bin`.

To move an installed desk to a user, stop it, hand over the room and drop the
account in with an override:

```sh
sudo systemctl stop hotline
sudo chown -R agent:agent /var/lib/hotline/room
sudo chmod o+x /var/lib/hotline
sudo systemctl edit hotline    # then paste the four lines below
sudo systemctl start hotline
```

```ini
[Service]
User=agent
Group=agent
Environment=HOME=/home/agent
Environment=PATH=/home/agent/.local/bin:/home/agent/.npm-global/bin:/usr/local/bin:/usr/bin:/bin
```

A desk that runs as you runs its teammates' tools as you, with your home in
reach of whatever a teammate is allowed to touch. Use the service account on a
server that is shared or holds anything you would not hand an agent.

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

## Relay

A served desk is also a relay for desktops paired with it as owners, so a
phone reaches a desktop through the server instead of over a VPN. On the
desktop, add the server as an owner (Add a server, with `hotline pair --link`
from the server), then turn on Remote and
choose the server under **Relay**. The desktop dials out to the server and
stands in there; nothing listens on the desktop's network for it.

Phones paired after that get the relay address in the QR code. Phones paired
before learn it the next time they connect directly. A phone dials the relay
first and the desktop's own address after.

The relay joins a phone's socket to one the desktop dials back for it and
passes the Noise records between them as they are. The phone still pins the
desktop's key, so the server cannot read or alter the session; it can only
drop it. See [ADR 0002](adr/0002-a-served-desk-relays-sealed-records.md).
Each desktop may have 16 sessions through the relay at once, and a server
stands in for up to 64 desktops.

## Public Cloudflare tunnels

Enforce HTTPS at the Cloudflare edge. The desk only listens with TLS and
native clients already require an HTTPS URL; an edge redirect also prevents
plain HTTP requests from reaching the API. Noise continues to authenticate
the desk and device independently of the proxy certificate.

For `grizzly-tunnel.hotline.dev`, the reviewable rule is
[`packaging/cloudflare/grizzly-https-redirect.json`](../packaging/cloudflare/grizzly-https-redirect.json).
In Cloudflare's **Rules → Redirect Rules**, create a Single Redirect with:

- Match: `(http.host eq "grizzly-tunnel.hotline.dev" and http.request.scheme eq "http")`.
- Dynamic target: `concat("https://grizzly-tunnel.hotline.dev", http.request.uri.path)`.
- Status: **301**; **Preserve query string** enabled.

The JSON is one rule for the zone's `http_request_dynamic_redirect` ruleset.
Using the [Cloudflare Rulesets API](https://developers.cloudflare.com/rules/url-forwarding/single-redirects/create-api/),
append it to the existing ruleset (or update the existing rule with this `ref`);
do not replace the zone's other rules. This repository does not deploy that
ruleset automatically. The operator applies it during the tunnel rollout.
Other public tunnel hosts should use the same rule with their own hostname.
[Always Use HTTPS](https://developers.cloudflare.com/ssl/edge-certificates/additional-options/always-use-https/)
is an alternative when every hostname in the zone should redirect.

The match excludes HTTPS, so secure WebSocket upgrades are not redirected.
Keep the tunnel's origin connection pointed at the desk's **HTTPS** listener.
Do not add an origin redirect based on client-supplied forwarding headers.

After deployment, check both the root and a path with a query:

```sh
curl --silent --show-error --max-time 10 --output /dev/null --dump-header - \
  'http://grizzly-tunnel.hotline.dev/'
curl --silent --show-error --max-time 10 --output /dev/null --dump-header - \
  'http://grizzly-tunnel.hotline.dev/v2?redirect_probe=1'
curl --silent --show-error --max-time 10 --output /dev/null --dump-header - \
  'https://grizzly-tunnel.hotline.dev/'
```

The first two responses must be 301 with exactly the equivalent HTTPS
`Location`, including `/v2?redirect_probe=1`. The HTTPS root may answer the
API's 404, but must not redirect back to HTTP or loop. The query is only a
redirect probe; authenticated native routes intentionally reject queries.
Reconnect an already-paired native client over WSS, confirm a wire command
and viewer connection, and record the deployed version and rule ID with the
release. These are rollout checks, not claims that this rule is already live.

### Validate admission before rollout

Run `cargo test -p hotline-core remote::tests::admission_tests --locked` for
isolated tests using fresh device keys, temporary state and a local
TLS-terminating proxy. The tests exercise shared proxy-IP saturation,
reconnects, existing sessions and HTTP keepalive expiry without touching a
running desk. They do not model Cloudflare connection pooling.

For controlled Cloudflare staging, use a separate hostname and disposable
served desk with synthetic paired devices. Hold four idle HTTPS or stalled
Noise connections, reconnect a paired device, and confirm the existing
session still works. Repeat with HTTP keepalive requests beyond five seconds;
anonymous origin connections must expire and reconnects must recover. Verify
pairing and revocation as well, then tear down the staging desk. Record the
proxy configuration and observed connection counts. Never run this saturation
check against the production desk.

For the 0.30 release, verify the packaged application and server use the fixed
Rustls resolution with `cargo tree --locked --target all -i rustls` and run
`cargo audit`. The lockfile pins Rustls 0.23.45, which fixes
[RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html).
Record the installed version after upgrading the remote desk; merging the
lockfile change does not update an already-running binary.

## Pair a device

On the machine running the desk, as the service account or root:

```sh
hotline pair
# Or grant the existing limited phone seat explicitly:
hotline pair --companion
hotline devices
hotline revoke DEVICE_ID
```

`pair`, `devices`, `revoke`, `status` and `wire` find the served room
themselves. With no `--data`, no `HOTLINE_DATA_DIR`, and no desk running in
your own data folder, they use the room of the `hotline` service: the
`HOTLINE_DATA_DIR` in the unit's environment (`systemctl show hotline -p
Environment`), then `/var/lib/hotline/room`. If your account cannot read that
room's `door.json`, the command stops and prints the line to run as the
service's user, which is the unit's `User=` (a drop-in may change it):

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline pair --link
```

A room you name is always taken as given, so for a setup the lookup cannot
see, such as a room somewhere else, use that form yourself: set
`HOTLINE_DATA_DIR` or pass `--data`.

Scan the terminal QR with Hotline on the phone. The two-minute invitation
is single-use; the command waits and prints the device name and role.
Treat the QR and terminal scrollback as sensitive until it expires. Cancel
with Ctrl-C. A served desk does not offer the six-digit manual pairing
route, and its v2 claim endpoint is absent when no pairing window is open.
Removing every device does not open an owner bootstrap route: run `pair`
again explicitly. An owner has the same commands and subscriptions as the
local desk, including providers, grants and server paths. A companion keeps
the limited phone command set. Existing grants without a role remain owners.

The default command prints only the QR, never the link as text, so a casual
copy of the terminal does not carry the secret. `hotline pair --json` prints
one JSON payload to stdout; `hotline pair --link` prints only the link.
Both wait for a claim, cancellation or expiry, so an SSH caller must keep the
process alive after reading the first line. Completion and errors use stderr.
The link is `hotline://pair?p=<base64url(JSON)>`, without base64 padding. Its
payload fields are `version` (2), `url` (HTTPS), `deskKey` (base64url X25519),
`secret`, `role`, `expiresAt` (Unix milliseconds), and `name`. The existing
phone QR query keeps `v`, `u`, `k`, `s` and adds `r`, `e`, `n` for the same
role, expiry and name. All forms name the same single-use invitation.

## Check on it

```sh
hotline status
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
echo '{}' | hotline wire providers.list
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

## Desktop client library

`hotline_core::remote::client::pair(&payload, device_name)` uses the OS secret
store and returns `PairedDesk { desk_id, name, url, desk_key }`. Tests and shells
with an explicitly chosen store use `Client::new(store).pair(...)`. Keep the
registry entry; it contains no private key or invitation secret.

`remote::bridge::Bridge::start(&desk).await` returns a bridge whose public
`origin` is `http://127.0.0.1:PORT` and whose `token` is independent of Remote.
Keep the Bridge alive for the desk's lifetime; dropping it closes its sockets.
The window connects to `/ws?token=TOKEN` and speaks the Door wire. Computer
viewers use `/computer/PERSONA_ID/ws`, with ordinary text and binary viewer
frames. Before each upgrade, the shell calls `bridge.viewer_token(persona_id)`
and passes `hotline-viewer.<token>` as the WebSocket subprotocol. The bridge echoes
it in the handshake. A token is single-use, expires after 30 seconds, and works
only for that persona on that bridge; reconnects need a fresh one. The owner
`?token=TOKEN` is accepted only on `/ws`, never on a viewer route. Viewer upgrades
require the single-use subprotocol token; query-string tokens and mixed owner/viewer
credentials are refused. Only the server holds the computer's bearer.

The bridge exposes a watch receiver in `state`. Its wire connection reconnects
with backoff from 250 ms to 30 seconds and resubscribes for fresh snapshots.
An interrupted command returns an uncertain-outcome error and is never replayed.
The shell should use the state to disable actions while the desk is unreachable.
A rejection authenticated by the pinned desk changes state to `revoked` and ends
reconnect attempts; pair again to restore access. A network failure alone never
sets `revoked`.

Owner file commands use absolute **server** paths:

| Command | Params | Result |
| --- | --- | --- |
| `files.browse` | `path` | canonical `path`, `parent`, `entries` with name/path/directory/size |
| `files.mkdir` | new directory `path` | created `path` |
| `files.download` | `path`, `offset` | name/mimeType/size/offset/base64 `data`/nullable `next`; at most 512 KiB |
| `files.upload_start` | exactly one of new destination `path` or plain filename `name` | `uploadId`, `offset`, destination `path` |
| `files.upload_chunk` | `uploadId`, `offset`, base64 `data` | next `offset`; at most 512 KiB |
| `files.upload_finish` | `uploadId` | destination `path`, `size` |
| `files.upload_cancel` | `uploadId` | no result |

`name` stages below `<desk-data>/uploads/<random>/<name>`; separators, traversal
and drive prefixes are refused. Finished staged files stay there for attachments.
Cancel or disconnect removes an unfinished file and its private staging directory.
Upload calls must stay on the socket that started them. Up to eight uploads can
be staged on it; cancellation or disconnect removes unfinished files. Finish
publishes a complete file without replacing an existing destination. Browse
refuses directories over 10,000 entries; select a more specific server path.
A completed upload's path can be passed to attachments. For room import,
create the destination tree with `files.mkdir`, upload its files, and pass the
server directory to `room.import`.
Existing `file.read` still reads sent files by message ID.

`computer.cookies.push` takes `personaId` and `transfer` containing `sourceId`
(a stable laptop ID), `browserId`, `profileId`, selected `domains` and `cookies`.
The shell reads the laptop browser and submits only the selected cookies. The
server checks the selection, drops expired cookies, and delivers them to the
teammate's computer through the existing import path. Values do not enter room
or tape records. Source identity keeps laptop imports separate from server-local
browser imports; the reply contains domain counts only. Transfers are bounded
to 8 MiB and 10,000 cookies. Companions cannot use these operator commands.

### Files in a remote computer viewer

The sealed viewer socket accepts text frames `{type:"files", id, op, ...}`.
Replies echo `id` with `{type:"files", id, ok:true, result}` or
`{type:"files", id, ok:false, error}`. Paths belong to the teammate's computer,
not the desk host. Both paired roles can use the viewer's computer files.

| `op` | Fields | `result` |
| --- | --- | --- |
| `list` | `path` (empty means `/home/agent`) | `path`, `home`, `entries:[{name,is_dir,size}]` |
| `download` | `path`, `offset` | `name`, `size`, `offset`, base64 `data`, nullable `next` |
| `upload_start` | new destination `path` | `uploadId`, `offset:0` |
| `upload_chunk` | `uploadId`, `offset`, base64 `data` | next `offset` |
| `upload_finish` | `uploadId` | `path`, `size` |
| `upload_cancel` | `uploadId` | `null` |

Chunks hold at most 512 KiB decoded. Handles belong to one viewer connection;
up to eight uploads and four unfinished downloads are retained. Cancel or
connection closure removes unfinished desk-side upload spools. Downloads retain
the upstream response between sequential chunks, since older computers have no
byte-offset endpoint. File requests use a bounded queue separate from screen
and control forwarding. A timeout invalidates unfinished transfers; restart them.

The desk calls the computer's `/files` and `/files/download` with its bearer in
an Authorization header. It never sends that bearer to the viewer. Uploads
require `/files` to advertise `x-hotline-upload-create-only: 1`; finish sends
`POST /files?path=...&create_only=true`. The computer must publish atomically
without replacing any existing destination, answering 409 on collision. Older
computers without this capability can list and download; uploads ask for an
update instead of risking another file. A failed finish may have reached the
computer, so check the destination before retrying.
