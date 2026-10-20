---
id: decision-4
title: Fixed-provider test origins are credential-scoped, loopback-only and compiled under test-support.
type: decision
status: current
claimKey: rig.provider-fixtures
kind: decision
stance: requires
subject: fixed-provider test origins
recheck: Revisit if production provider-origin configuration or the harness transport changes.
createdAt: "2026-10-20T14:13:25.122Z"
---

## Description
Fixed-provider test origins are credential-scoped, loopback-only and compiled under test-support.
The upgrade harness must exercise real provider signing and metering over the wire. Anthropic and ChatGPT have fixed origins, so their fixture override is enabled by the core's dev dependency, absent from shipping builds, and cannot be set through the wire or room settings. Unique fixture credentials isolate concurrent tests.
