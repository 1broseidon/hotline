---
id: task-12
title: "Desk: remote.devices marks the caller; hello carries desk kind, version and which endpoint is public"
column: backlog
position: 9
priority: medium
tags:
  - bro-160
createdAt: "2026-10-11T21:05:23.168Z"
description: |-
  Audit 2026-10-17, desk 0.39.0: none of this is done.

  - **(a) Mark the caller.** `remote.devices` returns plain grants (`wire/commands.rs:202`, `remote/v2.rs:103-112`). `RemoteDevice` is {id, name, pairedAt, role, publicKey} (`remote/mod.rs:64-75`). Add `current: true` on the grant the request came in on. The phone (`src/remote-devices.ts:3-8`) then labels "This phone" and warns plainly before revoking itself.
  - **(b) The owner hello carries kind and version.** The hello has {type, protocolVersion, desktopId, endpoints, mode, capabilities} (`wire/mod.rs:1154-1163`). Add `deskKind: "desktop" | "server"` and `version` (the Hotline version).
  - **(c) Mark the public endpoint** (for hotline-mobile task-25). The public URL is folded into `endpoints` unmarked (`remote/mod.rs:376-387`). Mark it, e.g. `publicEndpoint`, so any seat (companion included) can tell public from VPN. Today only an owner can learn it, via `remote.status.publicUrl`.
  - The optional summary read from the original task is out of scope; file it separately if wanted.

  All fields are optional, and an older phone ignores them. Run the contract sync with `make check`. Tests over the wire for each.
updatedAt: "2026-10-17T05:24:54.854Z"
subtasks:
  - id: task-12-1
    title: Mark the requesting device in remote.devices
    completed: false
  - id: task-12-2
    title: Expose desk kind/version and paired role in the owner hello
    completed: false
  - id: task-12-3
    title: Report Hotline Agent unavailable when no provider is connected
    completed: false
  - id: task-12-4
    title: Show desk reachability away from home in Settings › Remote, with the Relay row shown before a server is paired (from task-29)
    completed: false
  - id: task-12-5
    title: Add desk.status view (version, uptime, runtime, disk, failing logins/schedules, last error) for the phone banner and hotline status, each problem with its fix (from task-30)
    completed: false
parentId: epic-10
---

## Log
- 2026-10-14T01:44:31.126Z: [claude] Consolidated task-29 (Remote reachability/Relay row) and task-30 (desk.status/health) into this task as subtasks task-12-4 and task-12-5.
