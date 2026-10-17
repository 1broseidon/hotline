---
title: Use it away from home
description: Reach your desk from the phone when you're not on its Wi-Fi.
---

At home, the phone reaches Hotline on your computer over your Wi-Fi. To reach
it anywhere else, give the phone another way in. Either way, open Hotline on
the phone once while you're home afterwards, so it learns the new way in from
the desk.

## Your own VPN

Put the computer and the phone on the same private network, then restart
Hotline on the computer. The desk listens on that network too, and the phone
reaches it wherever you are. Any VPN that gives each device its own address
works. For example:

- **A mesh VPN such as Tailscale.** Install it on the computer and on the
  phone, signed in to the same account on both. Nothing needs to be exposed
  to the internet, and no port forwarding is needed.
- **WireGuard or your router's VPN.** Connect the phone to the network the
  computer is on.

The phone shows **Reachable over your VPN** under the desk once it has seen
the desk's VPN address.

## A Hotline server

Pair the desk with a [Hotline server](/docs/server/) and choose it under
**Settings → Remote → Relay** on the computer. The computer dials out to the
server, so nothing on your home network has to accept connections, and the
server can't read what passes through it: the phone only trusts the desk's
key. Phones paired after that get the relay in their QR code; phones paired
before pick it up the next time they connect at home.

