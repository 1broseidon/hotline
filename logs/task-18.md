---
id: task-18
title: "Cross-desk messaging: message_teammate, threads and replies over the link, under the recipient's grants"
priority: high
tags:
  - feature
  - bro-181
parentId: epic-3
createdAt: "2026-10-11T21:05:28.097Z"
description: |-
  Slice 4 of BRO-177, built on the link (slice 2) and the linked roster (slice 3). This is the slice where "talk to Lou on grizzly" works exactly like talking to Toad.

  ## Behaviour: identical to local

  * `message_teammate {to: "Lou", intent: ask|handoff, message}` to a linked teammate returns at once, like today (BRO-122). The message goes into the sender desk's durable exchange queue, crosses the link, and is delivered into Lou's desk's exchange. Lou's answer comes back the same way and arrives as its own message in the sender's conversation, matched by `requestId`.
  * `ask` runs in a side session on the recipient's desk with none of Lou's main context. `handoff` continues in Lou's main context and reports back, the same semantics as BRO-123.
  * **The standing private thread** between the two teammates exists on **both** desks. Each desk keeps its own copy of the thread, so either window, and the phone (BRO-110), can read it. Thread keys use the node-qualified ids.

  ## Grants: enforced where the recipient lives

  * The recipient desk applies the recipient's `allowedSenders` and the **operator's first-contact approval** exactly as for a local sender, using the sender's node-qualified id.
  * The approval card is raised on the recipient's desk. Since the operator is connected to it (the unified view, slice 1, or the phone), it shows up where they are, like any card. A sender waiting on approval sees the same "waiting for the person" state as locally.
  * The sender desk can't pre-approve, bypass or answer the approval.
  * Revoking the link fails queued and future messages with a plain sentence ("Lou is on grizzly, which isn't linked any more."), and pending approvals for that peer are withdrawn.

  ## Safety

  * A message from a linked desk is untrusted input, exactly like a local teammate's message. Nothing gains powers by crossing the link.
  * Message size and rate limits match local ones. Attachments and `send_file` in threads are out of scope for v0: the tool refuses with a sentence.

  ## Done when (the BRO-177 acceptance)

  With the laptop linked to grizzly:

  1. George asks Clementine to ask Lou to check the CI runners.
  2. Clementine calls `message_teammate` to Lou. On first contact, the approval card appears in George's view, and he approves once.
  3. Lou's reply arrives in Clementine's conversation.
  4. The thread is readable on the laptop, on grizzly and on the phone.

  Tests:

  * A scripted two-desk test covering ask, handoff and reply across the link.
  * The laptop offline mid-exchange: the reply is queued and delivers on reconnect.
  * Messages fail cleanly after revocation.
  * `allowedSenders` refusing a linked sender.

  Migrated from Linear BRO-181: https://linear.app/1broseidon/issue/BRO-181/cross-desk-messaging-message-teammate-threads-and-replies-over-the (2026-10-11).
updatedAt: "2026-10-13T07:31:41.610Z"
completedAt: "2026-10-13T07:31:41.610Z"
---

## Log
- 2026-10-13T07:31:38.577Z: [claude] On dispatch/epic-3: an ask or handoff to a linked teammate waits in the sender's exchange_pair, crosses when the queue takes it up, and runs on the recipient desk under that desk's allowedSenders (by qualified id) and a card on the recipient's tape; the sender's desk can't answer the card. The result returns over the link, kept until it arrives, and each desk keeps its own copy of the pair thread. Unlinking fails pending messages with "X is on D, which isn't linked any more.", withdraws cards and ends grants, and a stop on the recipient's desk answers the sender. Tests cover ask, handoff, reply, offline both ways, deny, revoke, ambiguity and bounds. Not done: the sender does not see a "waiting for the person" state while the other desk's card is open (residual in the ADR).
