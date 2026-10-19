---
id: task-89
title: "macOS shell sandbox: /usr/bin developer shims (python3, git, make) may hang when xcode-select points at Xcode"
column: todo
position: 6
priority: medium
tags:
  - sandbox
  - macos
  - shell
createdAt: "2026-10-16T22:26:28.277Z"
parentId: epic-14
updatedAt: "2026-10-19T00:46:15.778Z"
relatedFiles:
  - crates/hotline-core/src/tools/shell/macos.rs
  - crates/hotline-core/tests/processes.rs
  - .github/workflows/ci.yml
contract:
  status: ready
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - cargo test -p hotline-core --test processes
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:46:13.442Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:46:15.778Z"
assignee: claude
---

## Description
Found cutting 0.39.0 (2026-10-16). On GitHub's macOS runners, `python3 -m http.server` started from a teammate's sandboxed shell stayed alive but never listened and printed nothing, for 15 s, on all three macOS jobs; the same test passes on George's Mac (Command Line Tools installed) and on Linux, and a perl server passes everywhere. Suspected cause: /usr/bin/python3 (and git, make, clang…) are xcrun shims that find the toolchain through xcode-select (/private/var/db/xcode_select_link), which the Seatbelt profile in tools/shell/macos.rs may not let them read, so on a Mac with only Xcode they fall back to a missing default and hang. Confirm with an ignored test or a CI probe: run `/usr/bin/python3 -c 'print(1)'` and `/usr/bin/git --version` in the sandbox on a runner and on a Mac without CLT. If confirmed, allow reading the xcode-select link (read-only) and the developer dir it names, with a test.

0.40 triage (2026-10-18): Probe first: run `/usr/bin/python3 -c 'print(1)'` and `/usr/bin/git --version` inside the macOS shell sandbox on a GitHub macOS runner (a workflow_dispatch probe job). The sandbox already sets DEVELOPER_DIR (tools/shell/macos.rs:71-82), so the xcode_select_link theory may be wrong. Fix what the probe shows, with a test. tests/processes.rs:28 still uses python3.

## Log
- 2026-10-18T23:54:09.907Z: [claude] Lead: you cannot run a GitHub runner from your worktree, so don't fix anything on a guess. Ship (1) a macOS-only #[ignore] test in tests/processes.rs that runs `/usr/bin/python3 -c 'print(1)'` and `/usr/bin/git --version` in the real sandbox with a short timeout and says which one hung, and (2) a workflow_dispatch probe job in ci.yml that runs that test with --ignored on macOS runners. Change the Seatbelt profile only if you can reproduce the hang on this Mac, e.g. with DEVELOPER_DIR unset or pointed at a missing Xcode. Otherwise say in your notes that the fix waits on George's probe run.
