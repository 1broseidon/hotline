---
title: Hotline on a server
description: Run the desk on a Linux machine with no window, and reach it from your phone.
---

`hotline serve` runs the desk with no window: the same room, teammates,
schedules and computers as the desktop app, on a Linux machine you reach over
SSH. Your phone is the window. Teammates keep working while nobody is
connected, and the phone picks up where they are when you open it.

A server is not self-explanatory the way the desktop app is, so this section
goes in order:

1. [Install](/docs/server/install/): the binary, the account it runs as, and a
   systemd unit that listens on one address.
2. [Pair your phone](/docs/server/pair/): `hotline pair` prints a QR code and
   your phone scans it.
3. [Connect a model](/docs/server/models/): a one-time step on the server, so
   your teammates have something to run on.
4. [Teammates and computers](/docs/server/computers/): add teammates from
   the phone, and give one a computer.
5. [Day to day](/docs/server/operate/): status, logs, restarts, updates, and
   what lives where.
6. [Troubleshooting](/docs/server/troubleshooting/): what the messages mean.

## What you need

- A Linux machine, x86_64 or arm64, with systemd.
- An address your phone can reach: the same network or VPN, or a public
  address or name.
- Docker or Podman, only if a teammate will have a computer.
- Node, if your teammates will run harnesses installed through `npx`.

## How the phone and the server trust each other

Pairing gives the phone the server's own key, from the QR code. Every
connection after that is a sealed channel (Noise) to that key, inside TLS.
TLS only carries the bytes, so a self-signed certificate is fine, and so is
a proxy in front. A server that cannot prove it holds the key from the QR
never gets a connection. No password or token travels over the network. The
server knows each paired phone by the phone's key, and forgets it when you
revoke it.
