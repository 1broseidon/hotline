---
id: task-217
title: "Slice 2: post_to_chat on the desk's MCP server for both drivers; tell_person and Escalate go"
column: todo
position: 13
priority: high
tags:
  - threads
  - chat
  - security
assignee: claude
relatedFiles:
  - crates/hotline-core/src/mcp/server.rs
  - crates/hotline-core/src/driver.rs
  - crates/hotline-core/src/driver/rig
  - docs/security.md
parentId: epic-20
createdAt: "2026-10-19T19:59:31.099Z"
dependsOn:
  - task-216
  - task-218
updatedAt: "2026-10-19T20:02:52.541Z"
---

## Description
ADR 0008 §3 and §6 slice 2: post_to_chat(text, wake?) on Hotline's MCP server, same handler in-process for Hotline Agent; the gate (kind, modality, once a turn, one wake a turn, lease, 2,000 chars); a subagent the home agent launched is refused. security.md: default, grant source, enforcement, tests (every refusal row in the ADR's table on both drivers), residual risk. Removes the Escalate trait, escalate_next, the Rig-only tell_person and per-turn arming. An ACP quiet run that posts is proved.
