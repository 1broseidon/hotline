---
id: task-89
title: "macOS shell sandbox: /usr/bin developer shims (python3, git, make) may hang when xcode-select points at Xcode"
column: review
position: 6
priority: medium
tags:
  - sandbox
  - macos
  - shell
createdAt: "2026-10-16T22:26:28.277Z"
parentId: epic-14
updatedAt: "2026-10-19T00:56:48.766Z"
relatedFiles:
  - crates/hotline-core/src/tools/shell/macos.rs
  - crates/hotline-core/tests/processes.rs
  - .github/workflows/ci.yml
contract:
  status: delivered
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - cargo test -p hotline-core --test processes
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:46:25.646Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:56:48.766Z"
    duration: 623
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
- 2026-10-19T00:56:48.280Z: [claude] Probe shipped on dispatch/task-89 (6873f6e7); the profile is unchanged and the fix waits on George's probe run (Actions → ci → Run workflow; the shell-probe job, macos-26 + macos-15). Added: ignored macOS test `developer_shims_run_in_both_shells` in tests/processes.rs. It runs /usr/bin/xcode-select -p, /usr/bin/python3 -c 'print(1)', /usr/bin/git --version, PATH python3 and its socket.getfqdn("127.0.0.1"), with an 8 s watchdog each, first at machine reach and then at workspace reach (Seatbelt). It prints timings and fails naming any /usr/bin command that hung. Findings: (1) the 0.39.0 failure was NOT in the sandbox: the processes.rs Desk is reach "machine", which runs the shell unconfined with the runner's PATH python3, so the xcode_select_link theory cannot explain it. (2) The likelier cause: HTTPServer.server_bind calls socket.getfqdn(host) BEFORE listen(), and on this Mac getfqdn("127.0.0.1") does a real reverse-DNS lookup (it returns 1.0.0.127.in-addr.arpa). A slow resolver on a runner would give "alive, never listened", and the "printed nothing" is just stdout buffered into a pipe. The probe reports how long getfqdn takes. (3) Locally, with DEVELOPER_DIR forced to Xcode inside Seatbelt, the /usr/bin/python3 shim launches xcodebuild (first-launch check, logs fs-event errors) and takes ~1 s the first time, then is fast: no hang. I fixed the perl test's comment, which wrongly said python3 ran in the sandbox.
