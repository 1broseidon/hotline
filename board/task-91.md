---
id: task-91
title: "Core: a relay grant role that may only stand in and carry visits"
column: backlog
position: 45
priority: high
tags:
  - relay
  - security
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.415Z"
dependsOn:
  - task-90
updatedAt: "2026-10-17T05:37:03.997Z"
---

## Description
Core: a `relay` grant role.

- `DeviceRole` gains `Relay` (`remote/mod.rs:54`), with serde and an old-reader story: a desk that doesn't know `relay` must refuse such a grant, never treat it as owner.
- Relay grants may only open the relay control socket (`/v2/relay`), claim visits (`/v2/relay/accept`) and carry link-through (`/v2/link` via the relay). Every other seat purpose (owner commands, companion reads, door) is refused at admission with a clear error.
- `hotline pair --link --role relay` makes one. A relay-only server makes only relay grants, whatever is asked.
- `remote.devices` lists the role. Revocation works as today.

Tests: a relay grant stands in and carries a phone visit end to end, and is refused every owner and companion command and the door. An unknown role on disk is refused, not upgraded.
