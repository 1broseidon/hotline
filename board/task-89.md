---
id: task-89
title: "macOS shell sandbox: /usr/bin developer shims (python3, git, make) may hang when xcode-select points at Xcode"
column: backlog
position: 43
priority: medium
tags:
  - sandbox
  - macos
  - shell
createdAt: "2026-10-16T22:26:28.277Z"
---

## Description
Found cutting 0.39.0 (2026-10-16). On GitHub's macOS runners, `python3 -m http.server` started from a teammate's sandboxed shell stayed alive but never listened and printed nothing, for 15 s, on all three macOS jobs; the same test passes on George's Mac (Command Line Tools installed) and on Linux, and a perl server passes everywhere. Suspected cause: /usr/bin/python3 (and git, make, clang…) are xcrun shims that find the toolchain through xcode-select (/private/var/db/xcode_select_link), which the Seatbelt profile in tools/shell/macos.rs may not let them read, so on a Mac with only Xcode they fall back to a missing default and hang. Confirm with an ignored test or a CI probe: run `/usr/bin/python3 -c 'print(1)'` and `/usr/bin/git --version` in the sandbox on a runner and on a Mac without CLT. If confirmed, allow reading the xcode-select link (read-only) and the developer dir it names, with a test.
