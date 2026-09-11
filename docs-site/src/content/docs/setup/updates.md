---
title: Updates
description: How Toad checks for, downloads and installs a new version.
---

**Settings → Updates** shows the installed version and the latest one Toad
knows about. A packaged Toad checks GitHub every six hours while it runs;
**Check now** asks straight away. You choose when to install.

**Download, install and restart** fetches the package for this platform,
verifies its signature against the key built into Toad, installs it and
relaunches. A download can be cancelled; the native installer, once started,
cannot. Linux `.deb` and `.rpm` installs may ask for system privileges.

Toad refuses to install while any teammate is mid-turn, has a message
queued, or is in the middle of a session start. Once the room is idle it
holds new work until the restart completes. Schedules that come due in
between run after the relaunch.

Installing replaces the application only. The data directory, vault,
conversations and settings are untouched. See [Data and privacy](/reference/data/).

Development builds never check or install. Every release is also on the
[GitHub releases page](https://github.com/1Broseidon/toad/releases), with
notes.
