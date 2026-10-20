---
id: task-203
title: Calling the desk reaches the team lead, who answers the roster and hands named work straight on
column: backlog
position: 39
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/front_tools.rs
parentId: epic-21
createdAt: "2026-10-19T13:59:38.168Z"
dependsOn:
  - task-225
updatedAt: "2026-10-20T20:13:24.204Z"
---

## Description
Scenario: Calling the desk. George called today's desk call the worst of them: it runs a separate dispatcher with its own command table and narrates every step.

Done when:
- "Call the desk" reaches a call thread on the team lead, built and driven like any call (S5), with the lead's own voice front (S6).
- The lead answers roster questions itself, from a lookup.
- When the person names a teammate, the lead hands the words straight to them (message_teammate, handoff) and says who has it. Saying who is right here, because routing is the lead's job.
- No dispatcher model runs when the owner is clear. The dispatcher's desk role, the voice-dispatcher tape and its desk_command tool are gone.
- With no lead set, the desk call works as today, but knows the person's name and the roster.

Proved by scripted-front tests: a desk call with a lead answers a roster question itself, hands a named teammate's work straight to it, and never goes through the dispatcher when the owner is clear. The wire-level proof is in task-206.

## Log
- 2026-10-19T20:00:12.232Z: [claude] Now ADR 0008 slice 7 in epic-21: a desk call is a call thread on the team lead; removes the dispatcher's desk role, the voice-dispatcher tape and desk_command's reach into the wire's command table.
- 2026-10-19T20:02:41.594Z: [claude] From epic-17's lead: the lead's grant (hand work to teammates on a desk call without first-contact approval; stopping or reassigning others stays the person's) moved here from task-202, which has no desk-call caller in 0.41. It follows docs/security.md: default, grant source (the roster's lead from task-202), enforcement points, tests allowed and denied, residual risk.
