---
id: task-230
title: "S0: narrow Hotline's Rig seam and pin today's behaviour (still on 0.42)"
column: todo
position: 2
priority: high
tags:
  - rig
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:42.743Z"
---

## Description
Report §6 S0. No dependency bump. Copy the report into docs/plans/rig-0.45-migration.md. One usage adapter (Rig Usage → Hotline counters, TokenUsage, pricing), one stream-event mapper in turn.rs, the preamble kept on Turn and prepended per round, build.rs split completion_model from agent_builder where 0.42 allows. Behaviour-pinning tests that must survive every bump: Anthropic usage totals (cache tokens priced once), request-body snapshots for cache markers (Anthropic cache_control, OpenRouter anthropic/), Copilot chat assistant content as a string, Ollama think reaching /api/chat, the Failure::classify table on real error shapes.
