---
title: Troubleshooting
description: What the messages mean, and what to do.
---

## "No teammate can run yet: connect a model provider"

`hotline status` says this on a fresh room. [Connect a model](/docs/server/models/),
or give your teammates a harness that is already signed in for the account
the desk runs as.

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

The desk runs harnesses as its own account, with the unit's
`PATH`. `systemctl show -p User --value hotline` names that account: you,
or the `hotline` service account
([Quick install](/docs/server/install/#quick-install)). Check that Node (for
`npx`) is on that `PATH`, and sign the harness in as that account. See
[Connect a model](/docs/server/models/).

## Browser: off

`journalctl -u hotline | grep Browser:` shows why the teammates have no
browser, in brackets:

| Why | What to do |
| --- | --- |
| `WebKitGTK is not installed (LIBRARY is missing)` | Run the installer again with `--browser`, adding `--force` if the server is already current. See [A browser for teammates](/docs/server/install/#a-browser-for-teammates). |
| `Xvfb is not installed (the xvfb package)` | Install `xvfb`, then `sudo systemctl restart hotline`. |
| `hotline-browser is not installed beside /usr/local/bin/hotline` | Put the same release's `hotline-browser` beside `hotline`: run the installer again, with `--force` if the server is already current, or copy it from the release archive ([Manual install](/docs/server/install/#install-it-as-a-service)). |
| `hotline-browser is not this desk's version …` | The same: the two binaries must come from one release. |

## "Your browser stopped again soon after it started"

The teammate hears this when its browser keeps dying: the desk waits before
starting it again, from one second doubling up to a minute, and the message
says how many seconds are left. A browser that ran three minutes before it
stopped starts the count over. The reason is in the log, on lines beginning
`[browser]`:

```sh
journalctl -u hotline | grep '\[browser\]'
```

## Another desk has the room

`serve` refuses to start while the desktop app or another `serve` has the
same room open. Stop the other one. Don't delete `desk.lock` by hand.
