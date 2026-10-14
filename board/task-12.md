---
id: task-12
title: "Desk: remote visibility — which device is asking, reachability away from home, and desk health"
column: todo
position: 14
priority: medium
tags:
  - bro-160
createdAt: "2026-10-11T21:05:23.168Z"
description: |-
  These are two small wire additions the phone's new Settings wants:

  1. `remote.devices` **should mark the caller** (for example `current: true`). Then the phone can label "This phone" and refuse or strongly warn before revoking itself. Today every revoke gets the same generic confirmation.
  2. **The owner hello should carry the desk's kind (server or desktop) and its Hotline version.** The mockup's "Server · Owner" subtitle and "Hotline 0.29.0" pill were dropped because neither is on the wire.
  3. Optionally, **a cheap summary read** (counts plus anything needing attention across providers, tools, skills, secrets and schedules). The desk rows could then show counts and "needs sign-in" badges without fetching every area.

  Backend work for 0.30. Follow-up from BRO-153 (hotline-mobile #35).

  ---

  ## Merged in from BRO-116 (owner phone takes an empty headless room to a first turn)

  The rest of BRO-116 shipped (owner seat has the full command set; the phone connects providers and adds teammates). Left, and folded here:

  * `deskVersion` and `role` in the hello (`wire/mod.rs` \~1088 sends neither).
  * `backends.list` reports Hotline Agent as ready with no provider (`desk.rs` \~798, `unavailable: None`); it should say a provider is needed.
  * The empty room says only "Add one with the +"; a guided first step on the phone would help.

  Migrated from Linear BRO-160: https://linear.app/1broseidon/issue/BRO-160/desk-say-which-device-is-asking-in-remotedevices-and-the-desks-kind (2026-10-11).

  ## Status (2026-10-11 review)

  Also covers the hello fields and honest backends.list (merged from BRO-116). Its parent epic BRO-166 lives on the hotline-mobile board.
updatedAt: "2026-10-14T01:44:30.545Z"
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
---

## Log
- 2026-10-14T01:44:31.126Z: [claude] Consolidated task-29 (Remote reachability/Relay row) and task-30 (desk.status/health) into this task as subtasks task-12-4 and task-12-5.
