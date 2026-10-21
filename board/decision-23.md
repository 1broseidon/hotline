---
id: decision-23
title: Picker and inspector metadata caches belong to a desk connection and a watched room epoch.
type: decision
status: current
claimKey: ui.desk-read-cache
kind: decision
createdAt: "2026-10-21T10:55:44.352Z"
---

## Description
Picker and inspector metadata caches belong to a desk connection and a watched room epoch. Only backends, model efforts and voice capability metadata share reads; credential reads and per-persona computer status never enter the cache. Room model/credential changes, settings other than lastModelId, reconnect snapshots and loss of the last room reader invalidate it. Endpoint/token replacement uses a new connection identity, so data cannot cross desks.
