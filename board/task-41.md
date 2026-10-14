---
id: task-41
title: "Desk: fill slash_commands from ACP available_commands_update"
column: backlog
position: 24
priority: high
tags:
  - composer
  - acp
  - 0.37.0
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - crates/hotline-core/src/session/mod.rs
  - crates/hotline-core/src/wire/roster.rs
parentId: epic-4
createdAt: "2026-10-11T21:25:34.281Z"
updatedAt: "2026-10-14T20:05:24.734Z"
---

## Description
SessionSnapshot.slash_commands / SlashCommand { name, description, hint } exist in the contract but are always empty (session/mod.rs ~351, wire/roster.rs ~294). Read ACP `available_commands_update` in the driver and keep each teammate's list current on its roster row, sent only when it changes. Hotline Agent has none of its own.
