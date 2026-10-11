---
id: task-30
title: Desk health shows on the phone and in hotline status
column: todo
position: 32
tags:
  - bro-115
parentId: epic-1
createdAt: "2026-10-11T21:05:54.811Z"
description: |-
  Slice 1, item 4 of BRO-16.

  * A `desk.status` view: version, uptime, container runtime state, disk free, expired or failing logins, failing schedules, the last error.
  * The phone subscribes to it and shows a banner above the roster only when something needs action.
  * `hotline status` prints the same view, and says "not running" when `door.json` is stale. `hotline logs` is cut: use `journalctl -u hotline`.
  * Each problem comes with its fix: provider auth failure → Reconnect; AgentAuth → BRO-132's Sign in; a failing schedule → quiet or cancel; Docker down (only when computers are on); disk under 2 GB; listen address not up; a real certificate expiring within 14 days. An unreachable desk shows last seen and its last status.

  Acceptance: an expired provider login and a stopped Docker daemon each appear on the phone with what to do, and clear once fixed.

  Migrated from Linear BRO-115: https://linear.app/1broseidon/issue/BRO-115/desk-health-shows-on-the-phone-and-in-hotline-status (2026-10-11).

  ## Status (2026-10-11 review)

  `hotline status` is a first cut (crates/hotline-cli/src/door.rs ~172); no `desk.status` wire command, so no phone banner.
updatedAt: "2026-10-11T21:05:55.102Z"
---
