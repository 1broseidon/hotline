---
id: decision-3
title: Every Hotline request to Rig accepts an unknown finish reason as an ordinary stop
type: decision
status: current
claimKey: rig.unknown-finish-reasons
kind: decision
stance: requires
subject: accept_unknown_finish_reasons
packageVersion: rig==0.45.0
tags:
  - rig
createdAt: "2026-10-20T13:27:22.563Z"
---

## Description
Every Hotline request to Rig accepts an unknown finish reason as an ordinary stop: turns, single answers and voice (`accept_unknown_finish_reasons(true)`, set once in `driver/rig/build.rs` `RequestSettings`).

Why: Rig 0.44+ fails a reply whose finish reason is outside its list and marks it never to be replayed. A gateway passing its upstream's own reason through would fail a spoken answer that arrived whole, and drop a turn's reply from every later request while its tool results stayed in (orphaned results). Hotline 0.42 ended a reply on any reason it was not told was a truncation or a filter, and still does: a length or content-filter finish, or a reply the provider reports failed, runs none of its tool calls. Recorded in docs/design.md (the turn loop section).
