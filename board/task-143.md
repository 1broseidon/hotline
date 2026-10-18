---
id: task-143
title: "P2-34 (D): One boot that returns errors"
column: review
position: 32
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:58.025Z"
relatedFiles:
  - crates/hotline-app/src/lib.rs
  - crates/hotline-cli/src/serve.rs
  - crates/hotline-core/src/door.rs
  - crates/hotline-core/src/desk.rs
updatedAt: "2026-10-18T02:42:02.117Z"
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:24:05.170Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T02:42:02.117Z"
    duration: 1077
dependsOn:
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:03.139Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:23:40.711Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 34 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One boot that returns errors.** A core function that both `hotline serve` and the app call: room lock, backend, Desk, Remote, Door, and one token minter that never falls back to weak entropy. The desktop's four startup `expect`s and the CLI's runtime, signal and entropy `expect`s become errors at one outer handler, with an unsuccessful exit. The relay-only, file-store and drain differences stay as arguments (C13)

- Source: structure F13, task 20; errors F1, F3, tasks 1 and 3
- Files: `crates/hotline-app/src/lib.rs:66,307-362,570`, `crates/hotline-cli/src/serve.rs:161-282`, `door.rs:173`, `desk.rs`
- Collides with: none after 14
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T02:42:09.895Z: [claude] Done on dispatch/task-143, two commits. New `hotline_core::boot::boot(root, &RoomLock, Backend, Option<Served>) -> Result<Booted, String>`: claims the store, opens the file/native store, the Desk (skipped for relay-only), Remote (served or desktop), restores it, mints the Door token from getrandom (an error, never a fallback), binds and spawns the Door. `hotline serve` and the app both call it. The app's run() is now the one outer handler over start() -> Result: lock, store, desk, remote, door and the Tauri build/setup errors print "[startup] Hotline couldn't start: …" and exit 1 (the instance/store exits went through it too). The CLI registers SIGTERM/SIGINT before booting and returns errors for runtime and signals; door::runtime() returns Result for every client command, update.rs included. Deleted: Remote::open (no caller left), the app's rand token, the CLI's getrandom dependency (now a dev-dependency for its tests). Drain stays in serve.rs's shutdown, the app's exit is unchanged. Proofs: boot::tests (an unreadable remote.json stops the boot and is kept byte for byte; a booted Door listens behind a 64-hex token), and the cli harness `a_room_whose_remote_settings_cannot_be_read_is_not_served` (exit 1, no panic, no door.json, remote.json unchanged). Docs: serve.md (what a failed boot does), the module maps in development.md and design.md list boot.rs. make check, make verify, make precheck (7/7) pass. Left for later: (1) the app reports a startup failure only to stderr/logs/hotline.log, with no dialog, because before Tauri builds there is no window to show one in; errors.md's "Hotline couldn't start" dialog with selectable detail is still owed. (2) No injected failure for bind or for the shell setup, only remote.json. (3) Behaviour change: serve now records --capabilities in serve.json before the store claim, so a run with the wrong --store can still update serve.json before it is refused. (4) computer.rs new_token (uuid v4) and live.rs rand_ssrc (ignores getrandom errors; it is an RTP SSRC, not a secret) were left alone.
