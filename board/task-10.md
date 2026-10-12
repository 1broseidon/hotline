---
id: task-10
title: "Desk: requests.pending, schedule failures as facts, and no subscription cap that a big room exhausts"
column: todo
position: 12
priority: medium
tags:
  - bro-135
createdAt: "2026-10-11T21:05:21.341Z"
description: |-
  Slice 3 of BRO-16, alongside BRO-115 (health).

  * **Schedules:** the owner can cancel a schedule and set it quiet from the phone. Schedule failures are recorded as facts (attempt, error), not only logged, so health can show them.
  * **Devices:** list and revoke from the phone (owner only). Revoking cancels that device's unfinished logins so none of them commits late. A lost phone is recovered with `hotline devices` / `revoke` over SSH.
  * **Pending requests:** a `requests.pending` view, so an approval or human answer whose acknowledgement was lost resolves from the phone. It is never replayed blindly. This removes the "answer on the desktop" fallback in `mobile/src/Message.tsx`.
  * **Subscription budget:** the phone subscribes to a tape only while it's on screen, and the roster carries attention state. More than seven teammates must not exhaust the 8-subscription cap (`wire/mod.rs`).
  * **Companion "always allow":** refused in the handler. Today `session.answer_permission` from a restricted phone can persist `allowedSenders`.
  * **Computers:** the owner turns a teammate's computer on or off from the teammate card, with the desk validating it. The viewer and Stop already exist.

  Acceptance: twelve teammates all reachable; revoking a device mid-login leaves no credential; an approval whose acknowledgement was dropped resolves from the phone with no duplicate answer; each owner command is refused for a companion through the real handler.

  Migrated from Linear BRO-135: https://linear.app/1broseidon/issue/BRO-135/run-a-headless-desk-for-weeks-from-the-phone-schedules-devices-pending (2026-10-11).

  ## Status (2026-10-11 review)

  Done: phone schedules cancel/quiet, device list/revoke, computer on/off. Left: `requests.pending` ("Answer this on your desktop" still in Message.tsx ~454), schedule failures as facts, companion 8-subscription cap (wire/mod.rs ~1780).
updatedAt: "2026-10-12T01:56:58.740Z"
---
