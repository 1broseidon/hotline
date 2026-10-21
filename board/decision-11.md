---
id: decision-11
title: A desk lead's implicit local handoff grant belongs to the live desk call, not the teammate globally.
type: decision
status: current
claimKey: voice.lead-call-grant
kind: decision
createdAt: "2026-10-21T06:29:24.686Z"
---

## Description
A desk lead's implicit local handoff grant belongs to the live desk call, not the teammate globally. The core checks the current lead, call attachment and capability leases at use and again before queued work starts; saved requests cannot recreate that grant after restart. Work already started keeps the ordinary recipient lease and survives hang-up. This implements task-203 without making DM, direct-call, linked-desk or later background requests implicitly trusted.
