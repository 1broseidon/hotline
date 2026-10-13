---
id: task-16
title: "Desk-to-desk link: pair two desks for their teammates, with a durable, revocable link channel"
priority: high
tags:
  - feature
  - bro-179
parentId: epic-3
createdAt: "2026-10-11T21:05:26.454Z"
description: |-
  Slice 2 of BRO-177: the trust and transport that let two desks' teammates reach each other. The roster (slice 3) and messaging (slice 4) ride on it.

  ## Link vs connect

  Connecting (BRO-145) makes a **window** an owner client of a remote desk. Linking makes the two **desks** (the cores) trust each other. A link is:

  * **Deliberate.** It's the operator's action, from Settings › Desks on a connected desk: "Link with this desk". No new screen. A connected desk is not linked by default.
  * **Per pair, both sides consent.** The operator holds the owner seat on both desks, so one action can confirm on both. Each side records the link, and either side can revoke it.
  * **Narrow.** It's a new `Seat::Link`, or equivalent, that permits **only** the federation commands from slices 3–4: roster metadata for teammates, and message delivery into the durable exchange queue. No settings, no tapes, no files, no computers, no approvals. A test covers that everything else is `forbidden` (the same pattern as `voice_is_owner_only_through_the_real_handler`).

  ## Transport

  * Reuse the BRO-114/145 **sealed Noise channel and pinned keys**, with no new crypto. The desk that can reach the other opens the channel. For laptop↔server, the laptop's core dials out to the server, so the server needn't reach a laptop behind NAT. Once open, the channel carries traffic both ways.
  * **Durable and quiet.** Reconnect with backoff. A message for a desk that's offline waits in the sender desk's exchange queue (BRO-122) and delivers on reconnect. Nothing fails just because the laptop was asleep.
  * **Revocation** closes the channel, drops queued outbound messages with a plain sentence back to the sender, and removes the peer's teammates from the roster (slice 3). Revocation from either side takes effect on both.
  * Link state (connected, reconnecting, revoked) shows in the Desks section and in the unified view's desk filter (slice 1).

  ## Security

  Check this against BRO-167, the public tunnel hardening. A link grants **no** power over the other desk beyond messaging its teammates under their own grants. A compromised linked desk can at most send messages that the recipient desk's `allowedSenders` and first-contact approval still gate. The link and its revocation are room events on both desks.

  ## Done when

  * From the laptop, "Link with grizzly" in Settings › Desks establishes the link, and both desks show it.
  * The link survives the laptop sleeping and grizzly restarting.
  * Revoking it on either side ends it on both.
  * A `Seat::Link` peer gets `forbidden` for every command outside the federation set.

  Migrated from Linear BRO-179: https://linear.app/1broseidon/issue/BRO-179/desk-to-desk-link-pair-two-desks-for-their-teammates-with-a-durable (2026-10-11).
updatedAt: "2026-10-13T07:31:41.088Z"
completedAt: "2026-10-13T07:31:41.088Z"
---

## Log
- 2026-10-13T07:31:33.892Z: [claude] On dispatch/epic-3: desks.link/link_accept/unlink/links, with desk_link room records on both sides. The dialer uses its desk key on /v2/link, directly or through the relay, redials with jittered backoff, and treats an authenticated refusal as "ended". Seat::Link allows only the federation commands; the wire test proves every other command is forbidden. Link/Unlink is in Settings › Desks, and `hotline links`/`unlink` on the CLI. remote/tests/link_tests.rs covers linking, surviving outages and restarts, unlinking from either side, the key check, and the relay.
