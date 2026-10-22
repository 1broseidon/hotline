---
id: decision-32
title: Remote outboxes shed tool updates and coalesce pending roster rows before disconnecting; lag…
type: decision
status: current
claimKey: wire.phone-backpressure
kind: decision
recheck: When the phone supports a negotiated flow-control or history recovery protocol.
createdAt: "2026-10-22T01:15:24.517Z"
---

## Description
Remote outboxes shed tool updates and coalesce pending roster rows before disconnecting; lag recovery restores the full phone window.
The queue remains bounded at 256 frames. Tool events and ephemeral deltas can be evicted or dropped; chat, cards, snapshots and replies retain delivery or cause a disconnect if no expendable frame remains. Roster rows coalesce only within a subscription and teammate, preserving omitted catalogs and never crossing a required frame. Every tool event remains in the log. Recovery uses the full PHONE_WINDOW (200 lines, still shaped and byte-budgeted), even after a small opening window, and the log's atomic fold/broadcast boundary prevents replay of stale revisions. Frame-size limits and the write deadline remain unchanged.
