---
title: Desk browser
description: The web browser each teammate without a computer has on the desk, and how you watch it, take it over and size its pages.
---

A teammate with no [computer](/docs/setup/computer/) has a web browser of
its own on the desk. It opens pages, reads them, clicks, types and takes
screenshots in a hidden window, with no container or MCP server to set up.
Each teammate's browser keeps its own sign-ins, apart from yours and from
every other teammate's.

It runs in the desktop app on macOS 14 and later, Linux and Windows. A desk
with no window, such as `hotline serve`, and a Mac before macOS 14 have none,
and no teammate there is offered it. A teammate with a computer uses the
browser on its computer instead.

## Your browser or its own

A teammate opens what it reads or works on in its own browser. `open_link`
is for when you ask: it opens the link in your default browser, or in the
one you name ("open it in Brave"), and only while you are at the desktop
app.

## What it does

The teammate drives it with the `browser` tool:

| Action | What it does |
| --- | --- |
| `navigate` | Opens an http or https address and waits for it to load. |
| `text`, `links` | Reads the page as lines, with every link and control marked, or lists its links. A long page comes in parts. |
| `click_ref`, `fill`, `select`, `check`, `hover`, `press`, `scroll`, `wait_for` | Acts on the page as a person would. |
| `screenshot` | A picture of the page for the teammate, which you don't see. `full_page` takes the whole page, top to bottom, in up to four pictures. |
| `viewport` | Sizes the page. See [Page size](#page-size). |
| `back`, `forward`, `reload` | Moves through its history. |

`fill` can type a login from **Settings → Secrets** without the teammate
seeing it, and only on that login's own sites. See
[Sign-ins](/docs/setup/sign-ins/).

The browser tells sites it is Safari (Edge on Windows), so sites that turn
away unknown browsers let it in.

## The corner picture

While the teammate uses its browser, a picture of the page sits in the top
right corner of its conversation, with a line under it saying who is
browsing and where. It stays up 45 seconds after the teammate's last move,
and for as long as the browser waits on you, for example on an expired
certificate. **Hide** on that line puts it away until the teammate moves
again.

When the page is not the desk's own size, the picture takes the page's
shape and a tag names it: **Phone**, **Tablet**, **Wide**, or the exact size.

## Watch and take control

Click the picture to open the browser in a window of its own. The bar at
the top says who is driving and shows the address, with **Back**,
**Forward** and **Reload**.

**Take control** gives you the page: the teammate's actions wait, and you
click, type and move the page yourself. **Give it back** returns it. A site
on a certificate the browser doesn't trust opens only if you take control
and choose **Visit anyway**.

## Page size

`viewport` lays a page out and pictures it as another screen would:

| Size | Pixels |
| --- | --- |
| `phone` | 390 × 844 |
| `tablet` | 820 × 1180 |
| `desktop` | 1280 × 800, the desk's own |
| `wide` | 1920 × 1080 |

An exact size takes a width from 240 to 3840 and a height from 240 to 2400
pixels. Asking with neither, or `desktop`, goes back to the desk's own size.

## Local addresses

A page on this computer or the local network (`localhost`, `127.0.0.1`,
`192.168.x.x`, a `.local` name) opens only after you allow its host and port
for that teammate, on a card with **Allow**, **Always allow** and **Deny**.
Addresses you always allowed are listed in the teammate's pane under
**Permissions → Local addresses**, each with **Remove**; a removed one asks
again next time. Hotline's own ports never open, whatever you answer.

## Limits

- A page never gets the microphone or camera.
- An upload field opens a file picker only while you have control.
- On Windows the teammate's pointer isn't drawn over the page, and the
  browser offers no passkeys, Windows Hello included.
