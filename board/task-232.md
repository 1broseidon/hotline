---
id: task-232
title: "S2: Rig 0.44 — constructors, assistant messages, typed options, request-level caching"
column: todo
position: 4
priority: high
tags:
  - rig
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:43.722Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T06:51:47.095Z"
---

## Description
Report §6 S2 and §3.8. Constructors for CompletionRequest/Usage/Text; Message::Assistant(AssistantMessage); ToolFunction.arguments as Map and invalid_arguments answered with an error; ToolName; Anthropic automatic caching and OpenRouter anthropic/ caching move to the request's .cache(CacheRetention::Short); Ollama to native_completion; _ arms on non-exhaustive enums; decide accept_unknown_finish_reasons for voice. Manual: an Anthropic multi-round turn shows cache_read > 0 on round 2.
