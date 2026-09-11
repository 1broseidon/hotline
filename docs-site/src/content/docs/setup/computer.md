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
| Docker (Docker Desktop or OrbStack) | Linux and macOS |
| Podman | Linux and macOS |
| Apple `container` | macOS only |

**Settings → Computer** lists every runtime, whether it is installed and
running, and what to do if it is not. Leave the choice on automatic and
Toad takes the first one that is ready, preferring one that runs rootless.

The desktop image is `ghcr.io/1broseidon/toad-computer`, pinned to one
version per Toad release, never `latest`. **Settings → Computer → Desktop
image** overrides the image for the room; a teammate can override it again
in its pane. The image and its agent live in their own repository,
[toad-computer](https://github.com/1broseidon/toad-computer).

## Turn it on

In the teammate's pane, **Computer**. The first start pulls the image, which
the conversation notes, and builds the container. From then on, the
container wakes with the teammate and stops with it. Stopping keeps the
container for the next start; removing it starts over. Thirty minutes idle
stops it; seven days idle removes it.

The teammate's workspace is mounted inside at `/home/agent/workspace`. Add
further folders as mounts in the pane, read-only if you like. **Processes**
caps how many processes the container may run.

## Watch and take over

While the computer is running, **Screen** in the title bar opens the
desktop in a window of its own. When the teammate needs you, for a login or
a tap the agent cannot do, its **Needs you** card opens the same screen.
What you type goes to the desktop and never through the agent. Mark the
card done and the desktop is the agent's again.

## What the container can do

Nothing in the image runs as root. Toad creates the container with every
capability dropped, no new privileges, memory and process limits, and its one
port on loopback only. Two named volumes persist packages and source between
starts.
