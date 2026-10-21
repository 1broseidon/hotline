---
id: decision-20
title: Live roster projections follow Log broadcasts; catalog omission is negotiated.
type: decision
status: current
claimKey: roster.live-projection
kind: decision
createdAt: "2026-10-21T09:23:25.021Z"
---

## Description
Log remains the sole folded-history cache (decision-19). Roster views retain only the last visible message, current tool and pending card ids, seeded atomically with their broadcast subscription; live side/peer sessions own their card projections. Lag replaces seed and receiver. A cold tape seed also reads raw activity order because folded tool ids cannot preserve overlapping completion order. Ordinary updates perform no tail or peer/side file reads. Only clients declaring rosterOptions receive omitted unchanged session.models/slashCommands; snapshots and first rows include both, empty arrays clear lists, and all other fields replace the previous row.
