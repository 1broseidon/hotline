---
id: task-231
title: "S1: straight to Rig 0.45 in one jump, with Hotline's own TokenStore over today's login dirs (no one signed out)"
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
dependsOn:
  - task-230
updatedAt: "2026-10-20T11:59:56.202Z"
---

## Description
Report §6 S1 and §3.2–3.7. The big break: every Client::builder() removed; HttpClientExt impls hold ReqwestClient; DynModel into turn::run with CompletionRequest::new and the system message per round; streaming via Item<StreamEvent>, finish(), partial(), drop to cancel; ProviderError in failure.rs; Usage as Option and DELETE the Anthropic cache fold (every_input_token, cache_beside_input); CallId/ToolName/Message::tool_result; DynamicTool closures without the ctx arg; voice dispatcher on prompt().stream(), CompletionCallEvent, settle in on_outcome (minimal); ChatGPT/Copilot via path-based Authenticator::new over today's files. S0's tests unchanged. Manual: one turn on each of Anthropic, OpenAI, ChatGPT and Copilot (existing sign-ins), OpenRouter anthropic/, Ollama, a custom server, xAI subscription; voice call metering; budget numbers equal to the 0.42 baseline for the same transcript.

## Log
- 2026-10-20T11:59:56.658Z: [claude] George, 2026-10-20 06:59 (call): go straight to 0.45 rather than one version at a time. This task now carries S1, S2 and S3 together (task-232 and task-233 are folded in): every change listed for 0.43, 0.44 and 0.45 in the report's §3 and §6, done once in its 0.45 form, plus S3's LoginDirStore TokenStore and per-credential authenticator registry so ChatGPT and Copilot users stay signed in. rig =0.45.0, rust-version 1.97.1. The three silent traps are mandatory: delete the Anthropic cache fold (every_input_token, cache_beside_input), re-prepend the system prompt every round, and ship the TokenStore in this same change. S0's pinning tests must pass unchanged. For the agent: run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done.
