---
title: Run a relay
description: A small server, a Raspberry Pi say, that only carries your phone to your desktop, and runs no agents.
---

A relay lets your phone reach the desktop at home without a VPN. The
desktop dials out to the relay and waits there; the phone dials the relay,
and the relay joins the two. It passes the sealed channel through as it is:
the phone trusts only the desktop's key, so the relay can't read or change
anything that crosses it, only drop it.

Any [Hotline server](/docs/server/) relays. If all you want is the relay,
serve it as one: a relay-only server (`--capabilities relay`) runs no room, no
teammates, no models and no provider keys, so it fits on a Raspberry Pi,
and a desktop paired with it may only stand in there. A stolen desktop key
can't run anything on it, or read anything.

## What you need

- A Linux machine with systemd, arm64 or x86_64. On a Raspberry Pi, a
  64-bit OS: Raspberry Pi OS bookworm (64-bit) or newer, or another distro
  with glibc 2.35 or newer.
- An address both the desktop and your phone can reach, on the same
  network, through a tunnel, or public.

No Docker, Node or model provider: a relay runs none of them.

## Install it as a relay

```sh
curl -fsSL https://hotline.dev/install | sh -s -- --server --relay
```

The installer downloads the server build for the machine's architecture,
checks it against the release's checksums, installs `hotline` to
`/usr/local/bin`, creates the `hotline` service account, and installs the
release's `hotline-relay.service` as `hotline.service`, the name
`hotline pair`, `hotline status` and `hotline update` look for. Nothing for
agents goes in: no one's home or CLIs on its `PATH`, and no room. It asks
for `sudo` when it needs it, and for the listen address and public URL, or
takes them as `--listen` and `--public-url`:

```sh
curl -fsSL https://hotline.dev/install | sh -s -- --server --relay \
  --listen 192.0.2.10:9443 --public-url https://relay.example:9443
```

Set `--listen` to the Pi's own address and a fixed port, and `--public-url`
to where the desktop and phone reach it, as for any server
([Listen address and TLS](/docs/server/install/#listen-address-and-tls)).
Open that port in the firewall and nothing else. Once the relay is up the
installer shows its status and, with a terminal attached, a link to
[pair your desktop](#pair-your-desktop-with-it). The unit runs:

```sh
hotline serve --store file --capabilities relay --listen 192.0.2.10:9443 --public-url https://relay.example:9443
```

Update it with `sudo hotline update`, or by running the installer again
with `--relay`. Without `--relay` the installer says the server runs only a
relay and stops, so a relay is never made a full server by an update; to do
that on purpose, see [Run agents there later](#run-agents-there-later).

Each [release](https://github.com/1broseidon/hotline/releases/latest)
carries `hotline-server_<version>_linux_aarch64.tar.gz` for a Pi (and
`_linux_x86_64` for a PC), with the binary and both units, for an install
by hand; the comment at the top of `hotline-relay.service` lists the steps.

Check it:

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline status
```

It says `runs    relay`, the address it answers on and how many devices
are paired. A relay keeps only its pairings and its keys in its data
folder; nothing of a room is created there.

## Pair your desktop with it

On the relay:

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline pair --link
```

On a relay, every invitation is a relay invitation, for a desktop that
stands in there and nothing else; `--companion` is refused, because a phone
has nothing to reach on a relay. The command prints the link and waits for
two minutes. Paste it into the desktop, either place:

- **Settings → Remote → Relay servers**, then **Add a relay**, and **Pair**.
  The relay is listed there.
- **Add a server**. The window says **Added *name* as a relay.** and keeps
  it as a relay, never as a desk. If no relay is chosen yet it chooses this
  one; if one is, it leaves that choice alone and points you to
  Settings › Remote › Relay.

The terminal on the relay says **Paired *name* as relay.**

On a server that also runs agents, `hotline pair` makes an owner, who can run
the server's teammates. For a desktop that should only use that server as a
relay, run `hotline pair --relay --link` instead.

## Choose it

On the desktop, turn on **Remote access** in **Settings → Remote** and choose
the relay under **Relay** (relays paired only as relays are marked **Relay**). The row then says **Phones also reach this desk through *name*.**

Phones paired after that get the relay in their QR code. Phones paired
before pick it up the next time they connect to the desktop directly. A
phone tries the relay and the desktop's own address, so it works at home
and away. Desktops [linked](/docs/server/link/) with this one reach it
through the relay the same way.

Pair your phones with the desktop, never with the relay: a phone that
scans a relay's code is told it is for Hotline on a desktop, and no pairing
is made.

## A server you already paired as its owner

If your desktop was paired with a server as its owner only to use its relay,
and you have no teammates there, Settings → Remote offers, once that server
is chosen under **Relay**: **Use *name* only as your relay?** **Use only as relay** narrows this desktop's own pairing to
a relay pairing in place: the desktop stands in again straight away, the
server leaves your list of desks, and from then on this desktop can't run or
read anything there. **Keep as desk** changes nothing, and the offer does
not come back. A server you have teammates on is never offered.

Nothing narrows a pairing but that button, and the relay pairing can't widen
itself: to make the desktop an owner again, run `hotline pair` on the
server.

## What it costs to run

Measured idle with nothing paired, on an Apple M5 Max (aarch64, macOS 27),
release build: a relay holds **14 MB** of memory and used under 10 ms of
CPU in a minute. The same binary running agents on an empty room holds
25 MB. Those figures are from a Mac; the footprint on a Raspberry Pi has not
been measured yet.

## See and revoke desktops

```sh
hotline devices
hotline revoke DEVICE_ID
```

Revoking a desktop ends its place on the relay at once and drops the visits
it was carrying. `hotline links`, `unlink` and `wire` say the server runs
only a relay.

## Run agents there later

The mode is kept with the data, in `serve.json` in the data folder, and the
unit's `--capabilities` sets it. To make the relay a full server, change the
flag to `agents,relay`:

```sh
sudo systemctl edit --full hotline     # --capabilities agents,relay
sudo systemctl restart hotline
```

The log says `capabilities: relay → agents,relay`, and `hotline status` says
`runs    agents, relay`. Every pairing is kept with its role: the desktops
paired as relays keep standing in, with no pairing again, and still can't
reach the server's teammates. The room opens for the first time, as a new
server: no teammates, no models, no provider keys. Set it up as any
[new server](/docs/server/install/): add to the unit's `PATH` what teammates
need, [pair your phone](/docs/server/pair/) and
[connect a model](/docs/server/models/). A desktop that should use the
teammates runs `hotline pair` there again and is added as an owner, which
replaces its relay pairing.

Going back to `relay` leaves the room on disk, unopened and unchanged, and
keeps every pairing; an owner desktop may still stand in on it.
