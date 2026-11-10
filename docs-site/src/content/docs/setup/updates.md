---
title: Updates
description: How Hotline checks for, downloads and installs a new version.
---

**Settings → Updates** shows the installed version and the latest one Hotline
knows about, with its release notes. A packaged Hotline checks GitHub every
six hours while it runs; **Check now** asks straight away. When a new version
is out, a small **Update available** card also appears in the window's
bottom-right corner; **Update…** opens Settings → Updates, and closing the
card hides that version until a newer one arrives. You choose when to
install.

## Stable and Beta

**Track** chooses which releases this desk is offered. **Stable**, the
default, gets final releases. **Beta** also gets the betas that come before
a final release, such as `0.49.0-beta.1`. A beta is a real build of the next
version, signed the same way, that has had less use.

A desk on Beta is offered each beta and then the final release, so it ends
up on the same version as everyone else. Switching back to Stable never
installs an older version: the desk keeps the beta it has until a newer
final release is out, and the Track row says which build that is.

**Update and restart** fetches the package for this platform,
verifies its signature against the key built into Hotline, installs it and
relaunches. A download can be cancelled; the native installer, once started,
cannot. A Linux `.deb` or `.rpm` install asks for your password in the
system's own prompt. If you dismiss it, nothing changes. If the install has
not finished after ten minutes, Hotline stops waiting and says so. You can
also install any release by hand from the releases page.

Hotline refuses to install while any teammate is mid-turn, has a message
queued, or is in the middle of a session start. Once the room is idle it
holds new work until the restart completes. Schedules that come due in
between run after the relaunch.

Installing replaces the application only. The data directory, keychain entries,
conversations and settings are untouched. See [Data and privacy](/docs/reference/data/).

Development builds never check or install. Every release is also on the
[GitHub releases page](https://github.com/1Broseidon/hotline/releases), with
notes.
