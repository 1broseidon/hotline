---
title: Pictures
description: Show a teammate a screenshot, have it draw an image, and give it a face.
---

## Show a teammate a picture

Paste a screenshot into the message box, drop an image on the window, or
use **+** (**Attach a file**). On the phone, **+** offers **Photos**,
**Camera**, **Files** and **Paste image**.

What the teammate sees depends on what runs it:

- **Hotline Agent** is sent the picture itself, along with the file's path.
  Hotline prepares a copy first: turned the right way up, flattened onto
  white, at most 2000 pixels on a side, saved as a JPEG of at most 1 MB.
  Your original file is not changed.
- **Claude Code, Codex and the other harnesses** are given the file, and
  the harness decides whether its model looks at it.

Up to four pictures go with one message, and the teammate keeps the four
most recent pictures in view (three on Groq). Older ones remain as files it
can open again.

A picture Hotline cannot prepare still reaches the teammate as a file, with
a note saying why: larger than 20 MB, more than 40 megapixels, or a format
it cannot read. HEIC is one of those; convert it to JPEG first. GIF and WebP
are read as a single still frame.

Not every model can see pictures. One that cannot ends the turn with an
**Input type unsupported** card; switch the teammate to a model that can,
from the picker in its header.

## Have a teammate draw

Every teammate can make pictures with the `generate_image` tool, once a
provider that draws is connected. Ask for "a diagram of the deploy flow as
a sketch", or for an edit of a picture you attached. The image is saved in
the teammate's workspace and posted in the conversation.

Pictures are drawn by the first connected provider that can draw, or the one
you choose in **Settings → Providers → Use for → Images**:

| Provider | Connects with |
| --- | --- |
| OpenRouter | sign-in or API key |
| OpenAI | API key |
| Google | API key |
| xAI (Grok Imagine) | API key, or a Grok subscription sign-in |
| ChatGPT | your ChatGPT subscription sign-in (experimental) |

A ChatGPT or Grok subscription is chosen ahead of paid keys when you have not
picked one, and a subscription that fails falls back only to another
subscription, never to a paid key. ChatGPT drawing uses an internal Codex
route that OpenAI can change, so treat it as experimental.

Drawing on a key counts against the **Images** budget; drawing on a
subscription counts against that subscription's own limits. See
[Budgets](/docs/setup/providers/#budgets).

## A face for each teammate

When a provider that draws is connected, **New teammate** offers to draw
the teammate a picture from its name and goal. You can also ask a teammate
to "make an avatar for yourself", and it sets the picture on the roster
itself. Give it a photo instead and ask it to use that.

## Pictures a teammate sends you

A teammate can send you a picture with `send_file`, such as a chart it made
or a screenshot of its [computer](/docs/setup/computer/). It shows in the
conversation, and clicking it opens it full size in the window's own viewer.
