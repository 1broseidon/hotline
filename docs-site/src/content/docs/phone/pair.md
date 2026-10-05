---
title: Link a desk
description: Pair the phone with Hotline on your computer, or with a server.
---

The first time you open Hotline on the phone, it asks you to link a desk.
Later, to add another, open the desktop menu beside the Hotline name at the
top of the team list and choose **Add a desktop**.

## Your computer

1. In Hotline on your computer, open **Settings → Remote** and turn on **Remote
   access**.
2. Press **Link a phone** to show the QR.
3. On the phone, scan it. Or press **Copy link** on the computer and open that
   link on your phone.

The phone needs to reach the computer: the same Wi-Fi, your VPN, or a
server selected under **Relay**. The QR and link expire after two minutes
and work for one phone. Press **Link a phone** again if they expire.
**Paired phones** lists every phone, and **Revoke access** unlinks one.

Phones paired with the six-digit code before 0.33 scan the new QR once.
Their old entries say **Needs re-pair** and can still be revoked. A phone
already paired through the sealed QR keeps working.

## A server

On the server, `hotline pair` prints a QR code. Scan it with **Add a
desktop**. See [Pair your phone](/docs/server/pair/) for the details. A
server can be anywhere the phone can reach, including the open internet: the
phone only trusts the key in the QR code.

## Several desks

Link as many as you like. The desktop menu at the top of the team list
switches between them or shows every desk's teammates together.
**Desktop settings** in the same menu names each desk, reconnects it, or
forgets it.
