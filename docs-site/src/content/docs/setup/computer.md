---
title: The computer
description: A containerized Linux desktop a teammate drives, and you can watch and take over.
---

A teammate can have a computer: a Linux desktop in a container on your
machine, with a browser and a window manager, driven through tools. The
container is the machine; the agent is the operator. You can open the same
screen, watch, and take the keyboard when the agent asks you to.

## What you need

One container runtime:

| Runtime | Where |
| --- | --- |
| Docker (Docker Desktop or OrbStack) | macOS, Linux and Windows |
| Podman | macOS, Linux and Windows |
| Apple `container` | macOS only |

**Settings → Computer → Teammate computers → Runs on** lists every runtime,
whether it is installed and running, and what to do if it is not. Leave it on
automatic and Hotline takes the first one that is ready, preferring one that
runs rootless.

The desktop image is `ghcr.io/1broseidon/hotline-computer`. **Image**, in
the same place, chooses its release: **Newest** follows each new release of
the image, or pick one to pin it, or a custom image of your own. **Check now**
looks for a newer release. A teammate can use a different image, under
**Desktop** in its pane. The image lives in its own repository,
[hotline-computer](https://github.com/1broseidon/hotline-computer).

## Turn it on

In the teammate's pane, under **Computer**, switch on **A computer of its
own**, or turn on **Computer** under **Abilities** when you add the
teammate.

The first start downloads the image. A ring fills where the computer's
button will sit in the conversation's header. The teammate does not wait for
it: it answers straight away without its computer, can ask how the download
is going, and gets the computer once the image is down. If the computer
cannot start at all, for example because Docker is not running, the
teammate still answers and says why.

From then on the container wakes with the teammate and stops with it.
Stopping keeps the container for the next start; removing it starts over.
Thirty minutes idle stops it; seven days idle removes it.

## Inside

The teammate's working directory is mounted at `/home/agent/workspace`, and
its extra [folders](/docs/setup/teammates/#folders) under
`/home/agent/folders`. Under **Desktop** in the pane:

- **Memory** and **CPU** limits, on sliders that stop at what this machine
  has;
- **Processes**, the most processes the container may run (1024 when blank,
  0 for no limit; a parallel build needs more);
- **Mount a folder** adds another folder of yours, read-only if you like;
- **Updates** says which release the computer runs, and offers **Update**
  when a newer one is out. Updating recreates the container: the workspace,
  prepared environments and the browser profile come back with it, while
  anything installed into the container outside them is gone.

## Sign the teammate in

A teammate should not need your password to use a site. There are three ways
to give it access without showing it one:

- **Browser → Bring over browser cookies** copies the cookies for the sites
  you tick from a browser on this computer into the teammate's browser, so
  it starts signed in. You pick the browser, the profile and the exact
  sites. On a Mac, if macOS keeps Hotline out of a browser's data, the
  picker says so and opens **Privacy & Security → Full Disk Access**.
- **Settings → Secrets** keeps logins (sites, username, password and an
  optional two-step code) and variables (a key or token that becomes an
  environment variable). Tick which ones a teammate gets under **Secrets** in
  its pane. The computer types a login only on that login's own sites, and
  nothing stored is ever shown again, to you or to the teammate.
- **Browser → passkeys** makes a passkey that belongs to the teammate. Name
  it and the site, choose **Arm the computer**, then within ten minutes sign
  in to the site on its screen and add a passkey there (or ask the teammate
  to). The site's request waits for you to approve it in the conversation.

## Watch and take over

While the computer is running, the computer button in the conversation's
header (**Open the teammate's computer**) opens the desktop in a window of
its own. When the teammate needs you for a login or a tap the agent cannot
do, its **Needs you** card has **Open the screen**. What you type goes to
the desktop, never through the agent. Mark the card **Done** and the desktop
is the agent's again.

In the viewer, <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>V</kbd>
(<kbd>⌥</kbd> + <kbd>⌘</kbd> + <kbd>V</kbd> on a Mac) pastes this computer's
clipboard into the desktop.

A teammate can also send you a screenshot of its screen, or a file from its
computer, in the conversation.

## What the container can do

Nothing in the image runs as root. Hotline creates the container with every
capability dropped, no new privileges, memory, CPU and process limits, and
its one port on loopback only. Named volumes keep its home, its source and
its installed packages between starts.

Work threads and side threads on the same teammate take turns on its
computer. One that finds it busy is told which thread has it.
