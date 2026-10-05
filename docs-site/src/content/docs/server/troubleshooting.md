---
title: Troubleshooting
description: What the messages mean, and what to do.
---

## "No teammate can run yet: connect a model provider"

`hotline status` says this on a fresh room. [Connect a model](/docs/server/models/),
or give your teammates a harness that is already signed in for the `hotline`
account.

## "Pairing expired. Run `hotline pair` to try again."

A pairing QR or link lasts two minutes. Open the scanner on the phone first
(**Add a desktop**), then run `hotline pair`.

## The phone can't reach the server

- Is the phone on a network that can reach the `--public-url` address? A
  server on a LAN address needs the phone on that LAN or your VPN.
- Is the port open in the firewall?
- Is the desk up? `hotline status`, then `journalctl -u hotline`.

## "Update the Hotline app to link to this server"

The phone is running a version from before servers. Update it from TestFlight.

## An old QR says the desk needs updating

Pairing through the six-digit code and older QR format ended in 0.33.
Update the desk, then open **Settings → Remote → Link a phone** or run
`hotline pair` to create a new QR. Scan it once with the updated phone.

## "This phone is no longer paired with the desktop"

The server doesn't know this phone any more: it was revoked, or the room
was moved to a new store. Run `hotline pair` and scan again.

## "The desk could not prove it is the desk you paired with"

Whatever answered at that address doesn't hold the key from the QR code the
phone scanned: a different machine, or a room recreated from scratch. If you
rebuilt the server on purpose, pair again. If you didn't, don't.

## A teammate's harness can't sign in or can't be found

The desk runs harnesses as the `hotline` account with the unit's `PATH`.
Check that Node (for `npx`) is on that `PATH`, and sign the harness in as
the `hotline` account. See [Connect a model](/docs/server/models/).

## Another desk has the room

`serve` refuses to start while the desktop app or another `serve` has the
same room open. Stop the other one. Don't delete `desk.lock` by hand.
