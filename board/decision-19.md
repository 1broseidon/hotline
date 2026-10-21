---
id: decision-19
title: Log owns the shared fold and chapter/activity cache under its writer lock.
type: decision
status: current
claimKey: store.fold-cache
kind: decision
holdsWhile: Log is the only live writer; imports precede opening it.
recheck: Revisit the budget or lock granularity with measured workloads; external writers would require explicit invalidation.
createdAt: "2026-10-21T09:15:25.135Z"
---

## Description
Log owns the shared fold and chapter/activity cache under its writer lock. The room is resident; other streams are bounded by 64 MiB estimated retained memory and 64 entries, evicted by read recency. Appends update revisions in their original slots before publication; rewrites invalidate. Failure-sensitive reads still inspect disk, and partial or unterminated recovery files stay cold. Keep one writer and unchanged event/segment formats so restart rebuilds the same state.
