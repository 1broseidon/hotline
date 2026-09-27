---
title: Pair your phone
description: Scan the server's QR code with Hotline on your iPhone.
---

On the server, as the service account:

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline pair
```

It prints a QR code and waits. In Hotline on your phone, open the desktop
menu beside the Hotline name at the top of the team list, choose **Add a
desktop**, and scan the code. The terminal says **Paired iPhone as owner.**
and the server shows up on the phone's team list.

- The code is good for two minutes and for one phone. Scanning it with a
  second phone does not pair that phone too. Run `hotline pair` again for
  each phone.
- The code carries a one-time secret. Treat the QR and your terminal's
  scrollback as sensitive until it expires. Ctrl-C cancels it.
- Nothing else can pair: there is no six-digit code and no pairing address
  while no `hotline pair` is waiting.

If your phone says **Update the Hotline app to link to this server**, it is
a version from before servers. Update it and scan again.

## Owner and companion

`hotline pair` pairs an **owner**. `hotline pair --companion` pairs a
companion. Today both can do the same things on the phone: chat, answer
cards, add, rename and remove teammates, and open a teammate's computer.
Neither can change what a teammate may reach; that stays on the server.

## See and revoke phones

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline devices
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline revoke DEVICE_ID
```

Revoking closes that phone's connections at once. The phone then says it is
no longer paired. Revoking every phone does not open pairing to anyone: run
`hotline pair` again on the server.
