---
title: Sign-ins
description: Logins, passkeys and your shared cookies, how a teammate asks to be signed in, and which of your browsers Hotline may read.
---

A teammate should not need your password to use a site. What it signs in
with sits under **Secrets** in its pane, and applies to whichever browser it
has: its [desk browser](/docs/setup/browser/) or its
[computer's](/docs/setup/computer/). The teammate never sees a password or a
cookie.

## Logins and passkeys

**Settings → Secrets** keeps logins (sites, username, password and an
optional two-step code) and variables (a key or token that becomes an
environment variable on a computer). Nothing stored is shown again, to you or
to a teammate.

Under **Secrets → Logins and passkeys** in the teammate's pane, tick the ones
it gets. Its browser types a login only on that login's own sites. A
variable needs a computer's shell.

To make a passkey that belongs to the teammate, name it and the site, then
choose **Arm the browser** (or **Arm the computer**). Within ten minutes,
sign in to the site in its browser and add a passkey there, or ask the
teammate to. The site's request waits for you to approve it in the
conversation.

## Cookies

Your cookies for a site, shared with the teammate's browser, sign it in as
you are signed in.

**Add Cookies**, under **Secrets → Cookies** in the teammate's pane, lists the
sites you are signed into across your browsers. Hotline reads that list only
when you open it. Press a site to share it. Each shared site is then a row,
"From Brave" or wherever it came from; open it for:

- **Keep fresh**: copies the site again when your browser's cookies for it
  change (it looks once a minute). Off by default.
- **Ends**: **1 day** or **Never** (the default).
- **Remove**: stops sharing.

The teammate is told which sites it holds cookies for at the start of its
next turn, never the cookies. Shared cookies follow the teammate when its
computer is turned on or off, read again from your browser each time.

## When it asks

A teammate that hits a sign-in wall asks on one card, with the site and its
reason:

- **Use Cookies** shares that site's cookies from your browser, named under
  the buttons. When several of your browsers have it, choose one. Shown only
  when one of them is signed in.
- **Sign In** opens username and password fields in the card. **Save to
  Secrets** (ticked) stores the login under the name shown; unticked, the
  teammate has it for the next hour, for this sign-in only.
- **Open Browser** opens the teammate's browser, or its computer's screen,
  for you to sign in yourself.

The close is **Not now**: it won't ask for that site again until you next
write to it. The card waits two hours, then folds to one line saying how it
ended. It also reaches a paired phone as a notification.

The desk never offers your cookies by itself; a site you are signed into is
shared only when you add it or answer a card.

## On a server

A server shares no cookies: it has no browsers of yours to read. Its
sign-in card offers **Sign In** and **Open Browser**, never **Use Cookies**,
and its Settings have no **Cookie Sources**. For a site with no login of its
own, the teammate asks you for a password or a shared secret instead.

## Cookie Sources

**Settings → Secrets → Cookie Sources** lists each browser and profile on
this computer, each with a switch. All start on. One switched off is never
read, for anything, not even its site names. On a Mac, a browser that macOS
keeps Hotline out of is named there; allow Hotline under
**Privacy & Security → Full Disk Access**, then reopen Hotline.

**In Use**, below it, lists every teammate holding your cookies, with the
sites, and opens that teammate's pane.
