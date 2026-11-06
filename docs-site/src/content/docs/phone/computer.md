---
title: Computer and browser
description: Watch a teammate's Linux desktop or its desk browser from the phone, and take over.
---

A teammate with a computer has a monitor icon in its conversation's header.
It turns green while the teammate is using its computer. Tap it to open the
screen. The computer is turned on at the desk: see
[The computer](/docs/setup/computer/), or
[Teammates and computers](/docs/server/computers/) on a server.

## Watching

<!-- TODO(screenshot, iOS simulator): a teammate's computer screen, watching, the fit button in the bar. -->

The screen opens watching: nothing you do reaches the computer. Pinch to
zoom and drag to look around. Double-tap, or the fit button, shows the whole
screen.

## Taking control

<!-- TODO(screenshot, iOS simulator): the computer screen after Take control, the keyboard and extra keys up. -->

Take control to use the computer yourself. Then:

- tap to click, hold to right-click, and use two fingers to scroll;
- switch to the **trackpad** for small targets: drag to move the pointer,
  and tap to click where it points;
- the keyboard button types on the computer, with a row of extra keys above
  the keyboard.

Hand it back, or close the screen, and the computer is the teammate's
again. Watching stops by itself after three minutes with nothing changing on
screen, so a forgotten viewer doesn't drain the phone.

## Desk browser

<!-- TODO(screenshot, iOS simulator): the chat header with the browser icon; then the desk browser full screen with its title, address and Take over. -->

<!-- TODO(phone 0.13, story 4, task-65; desk 0.48.1, hotline task-382): written from George's spec, not a build. Check the icon, when it shows, the screen's contents, and the words Take over and Hand back. -->

A teammate with no computer browses in its
[desk browser](/docs/setup/browser/). While it does, a browser icon sits in
the conversation's header, where the computer key would be. Tap it to open
the browser full screen, as the computer's screen opens: the page, its title
and address, and **Take over**.

After **Take over**, your taps, scrolling and typing go to the desk browser,
and so do **Back**, **Reload** and the address. **Hand back** returns it to
the teammate.

Taking over needs desk 0.48.1 or later. An older desk shows the browser to
watch only.
