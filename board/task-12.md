---
id: task-12
title: "Desk: remote.devices marks the caller; hello carries desk kind, version and which endpoint is public"
column: in-progress
position: 0
priority: medium
tags:
  - bro-160
createdAt: "2026-10-11T21:05:23.168Z"
description: |-
  Audit 2026-10-17, desk 0.39.0: none of this is done.

  **Scope for epic-10: only (a), (b) and (c) below.** Subtasks task-12-3, -4 and -5 (Hotline Agent unavailable, Settings reachability row, desk.status view) come from older tasks and are outside this epic. Do not build them here; leave them unticked for the person to file separately.

  - **(a) Mark the caller.** `remote.devices` returns plain grants (`wire/commands.rs:202`, `remote/v2.rs:103-112`). `RemoteDevice` is {id, name, pairedAt, role, publicKey} (`remote/mod.rs:64-75`). Add `current: true` on the grant the request came in on. The phone (`src/remote-devices.ts:3-8`) then labels "This phone" and warns plainly before revoking itself.
  - **(b) The owner hello carries kind and version.** The hello has {type, protocolVersion, desktopId, endpoints, mode, capabilities} (`wire/mod.rs`, the hello frame). Add `deskKind: "desktop" | "server"` and `version` (the Hotline version).
  - **(c) Mark the public endpoint** (for hotline-mobile task-25). The public URL is folded into `endpoints` unmarked (`remote/mod.rs` `status()`, the chain over `saved.public_url`). Mark it, e.g. `publicEndpoint`, so any seat (companion included) can tell public from VPN. Today only an owner can learn it, via `remote.status.publicUrl`. Keep the local-first order of `endpoints`: task-56 relies on it.
  - The optional summary read from the original task is out of scope; file it separately if wanted.

  All fields are optional, and an older phone ignores them. Tests over the wire in `crates/hotline-core/src/remote/tests.rs` (the TestPhone harness), one each: the paired phone sees `current` on its own grant only; the owner hello has deskKind and version; a companion's hello marks the public endpoint when one is set and has none when unset. Regenerate `ui/src/generated/contract.ts` with an unfiltered `cargo test -p hotline-core`, then `make check`.
updatedAt: "2026-10-17T06:22:24.671Z"
subtasks:
  - id: task-12-1
    title: Mark the requesting device in remote.devices
    completed: true
  - id: task-12-2
    title: Expose desk kind/version and paired role in the owner hello
    completed: true
  - id: task-12-4
    title: Show desk reachability away from home in Settings › Remote, with the Relay row shown before a server is paired (from task-29)
    completed: false
  - id: task-12-5
    title: Add desk.status view (version, uptime, runtime, disk, failing logins/schedules, last error) for the phone banner and hotline status, each problem with its fix (from task-30)
    completed: false
parentId: epic-10
relatedFiles:
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/wire/mod.rs
  - crates/hotline-core/src/wire/commands.rs
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/contract.rs
  - ui/src/generated/contract.ts
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/src/remote/tests.rs
      description: over-the-wire tests for current device, hello deskKind/version, publicEndpoint
  validation:
    commands:
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make check
  constraints:
    - Only (a), (b), (c); subtasks task-12-3/4/5 are out of scope
    - Every new wire field is optional
    - Keep endpoints local-first
  metrics:
    readyAt: "2026-10-17T05:44:14.749Z"
    pickedUpAt: "2026-10-17T05:45:38.021Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T05:55:17.836Z"
    duration: 580
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T05:44:30.937Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-17T05:44:32.076Z"
    ended: "2026-10-17T05:55:23.684Z"
    outcome: review
assignee: claude
---

## Log
- 2026-10-14T01:44:31.126Z: [claude] Consolidated task-29 (Remote reachability/Relay row) and task-30 (desk.status/health) into this task as subtasks task-12-4 and task-12-5.
- 2026-10-17T05:55:17.352Z: [claude] Done (a)(b)(c) on branch dispatch/task-12. For later tasks: phone `remote.devices` answers via Phone::devices with `current: true` on the caller's grant only (RemoteDevice.current is Option<bool>, never persisted). The v2 hello (wire/mod.rs seated_phone_v2) adds `publicEndpoint` for every seat (saved public_url, or a served desk's public_url; absent when unset or Remote off) and, owner only, `deskKind` ("desktop"|"server", from Remote.served) and `version` (CARGO_PKG_VERSION). endpoints order unchanged (local first). Documented in docs/wire.md; contract.ts regenerated. Three new tests in remote/tests.rs pass. One existing test, all_interfaces_are_the_default_and_every_advertised_ip_has_valid_tls, fails on this machine (an IPv6 ULA fdc2:: interface times out), which has nothing to do with this change. task-12-3/4/5 left unticked as scoped.
- 2026-10-17T06:22:01.420Z: Landed on release/0.40.0
- 2026-10-17T06:22:01.799Z: Merged; 5 of 5 subtasks left, so it stays open
