---
id: task-29
title: "Desk: show whether the desk is reachable away from home in Settings › Remote"
column: todo
position: 31
priority: high
tags:
  - bug
  - bro-261
createdAt: "2026-10-11T21:05:37.909Z"
description: |-
  Mobile pass 1 (OTA). From tester feedback, 2026-10-08.

  **What happened:** a tester paired an Android phone with their Windows desk on home Wi-Fi. Everything showed connected. Away from home nothing worked, and nothing had warned them.

  **Why:**

  * The pairing QR carries a single direct address, `endpoints.first()`, which is the default-route LAN IP (`hotline/crates/hotline-core/src/remote/v2.rs:114-151`).
  * The desk's hello sends every endpoint, including a Tailscale 100.x one. The phone keeps only the relay (`src/hello.ts:33-45`, `team-state.tsx:528`).
  * The green/yellow dot only answers "connected right now?", so at home it is green.

  **Pass 1 (phone):**

  * Keep every endpoint from the hello per desk and race them along with the relay. Tailscale on both ends then works without re-pairing.
  * Say per desk whether it will work away from home:
    * "Home network only": only private LAN addresses and no relay.
    * "Over your VPN": a VPN address such as 100.64/10 is known.
    * "Anywhere": through a relay.
  * A short "Use it away from home" explainer: Tailscale on both devices, or a relay.

  **Later (desktop release):** the same status in Settings › Remote, and the Relay row shown even before a server is paired.

  Migrated from Linear BRO-261: https://linear.app/1broseidon/issue/BRO-261/phone-say-whether-a-desk-is-reachable-away-from-home-and-try-every (2026-10-11).

  ## Status (2026-10-11 review)

  The phone side shipped in hotline-mobile #67 (endpoints raced, reach status, "Use it away from home"). Rescoped to the desktop remainder: the same reachability status in Settings › Remote, and the Relay row shown early. Retitled from "Phone: say whether a desk is reachable away from home, and try every address the desk advertises".
updatedAt: "2026-10-11T21:05:38.197Z"
---

## Log
- 2026-10-14T01:44:31.690Z: [claude] Consolidated into task-12 as subtask task-12-4 (remote visibility); closing as duplicate, not done.
