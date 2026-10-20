---
id: task-231
title: "S1: Rig 0.43 — providers, transport, streaming, errors, usage, hooks"
column: todo
position: 3
priority: high
tags:
  - rig
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:43.238Z"
---

## Description
Report §6 S1 and §3.2–3.7. The big break: every Client::builder() removed; HttpClientExt impls hold ReqwestClient; DynModel into turn::run with CompletionRequest::new and the system message per round; streaming via Item<StreamEvent>, finish(), partial(), drop to cancel; ProviderError in failure.rs; Usage as Option and DELETE the Anthropic cache fold (every_input_token, cache_beside_input); CallId/ToolName/Message::tool_result; DynamicTool closures without the ctx arg; voice dispatcher on prompt().stream(), CompletionCallEvent, settle in on_outcome (minimal); ChatGPT/Copilot via path-based Authenticator::new over today's files. S0's tests unchanged. Manual: one turn on each of Anthropic, OpenAI, ChatGPT and Copilot (existing sign-ins), OpenRouter anthropic/, Ollama, a custom server, xAI subscription; voice call metering; budget numbers equal to the 0.42 baseline for the same transcript.
