---
id: task-231
title: "S1: straight to Rig 0.45 in one jump, with Hotline's own TokenStore over today's login dirs (no one signed out)"
column: review
position: 0
priority: high
tags:
  - rig
assignee: claude
relatedFiles:
  - Cargo.toml
  - Cargo.lock
  - crates/hotline-core/src/driver/rig.rs
  - crates/hotline-core/src/driver/rig/turn.rs
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/rig/recovery.rs
  - crates/hotline-core/src/driver/failure.rs
  - crates/hotline-core/src/providers.rs
  - crates/hotline-core/src/providers/chatgpt.rs
  - crates/hotline-core/src/providers/copilot.rs
  - crates/hotline-core/src/providers/compat.rs
  - crates/hotline-core/src/providers/rig_login.rs
  - crates/hotline-core/src/desk.rs
  - crates/hotline-core/src/vault.rs
  - crates/hotline-core/src/pricing.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - docs/security.md
  - docs/design.md
parentId: epic-22
createdAt: "2026-10-20T06:51:43.238Z"
dependsOn:
  - task-230
updatedAt: "2026-10-20T13:27:38.719Z"
contract:
  status: delivered
  deliverables:
    - type: file
      path: Cargo.toml
      description: rig pinned at version =0.45.0, rust-version 1.97.1
    - type: file
      path: crates/hotline-core/src/providers/rig_login.rs
      description: LoginDirStore TokenStore over today's login dirs, per-credential lock and registry
    - type: test
      path: crates/hotline-core
      description: rig_login_* tests (Copilot dir migration, ChatGPT auth.json unchanged, single refresh-token spend)
    - type: docs
      path: docs/security.md
      description: credential storage under Rig 0.45
  validation:
    commands:
      - grep -qE '^rig = \{ version = "=0\.45\.0"' Cargo.toml
      - grep -A1 '^name = "rig-core"$' Cargo.lock | grep -q 'version = "0.45.0"'
      - grep -q 'rust-version = "1.97.1"' Cargo.toml
      - "! grep -rn 'every_input_token\\|cache_beside_input' crates/hotline-core/src"
      - make check
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_pin_')" -ge 6
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_login_')" -ge 3
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - S0's rig_pin_ assertions and expected values unchanged
    - Old Copilot login files removed only after copilot.json is written
    - Do not adopt Rig's resumable runs or committed-history streaming for the turn loop
  metrics:
    readyAt: "2026-10-20T12:04:00.044Z"
    pickedUpAt: "2026-10-20T12:29:25.431Z"
    reworkCount: 0
    deliveredAt: "2026-10-20T13:27:38.719Z"
    duration: 3493
dispatch:
  agent: claude
  model: opus
  effort: xhigh
  by: lead
  at: "2026-10-20T12:29:15.759Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T12:29:17.497Z"
decisions:
  - decision-3
---

## Description
Report §6 S1+S2+S3, §3.1–3.10 and §4 (docs/plans/rig-0.45-migration.md, landed by S0). One slice, because a dependency bump compiles only whole: the report's S2 and S3 are retired into this task, so everything they listed is done here, in its 0.45 form (skip 0.43/0.44 intermediate shapes).

Cargo: `rig = "=0.45.0"`, Cargo.lock, `rust-version = "1.97.1"`; the Windows precheck leg must pass on the new toolchain.

From 0.43 (§3.2–3.7): every `Client::builder()` replaced (`Anthropic::new`/`OpenAI::new`/vendor `new`, `OllamaConfig…connect`, `OpenAIConfig` for custom/xAI/zai/Copilot); `HttpClientExt` impls hold `ReqwestClient`, `http_client::Error` variants, `BoxedStream`; `DynModel<Completion>` into `turn::run`, `CompletionRequest` built per round with the system message re-prepended every round (the S0 preamble on `Turn`); streaming via `Item<StreamEvent>`, `finish()`, `partial()`, drop to cancel, `Truncated`; `ProviderError` in `failure.rs`; `Usage` as Option and DELETE the Anthropic cache fold (`every_input_token` in turn.rs, the dispatcher's `cache_beside_input`) because 0.43 input tokens already include cache reads and writes; `CallId`/`ToolName`/`Message::tool_result`; `DynamicTool` closures without the ctx arg; voice dispatcher on `prompt().stream()`, `CompletionCallEvent`, settle in `on_outcome` (minimal port: S6 rewrites metering); tests move from `ScriptModel` to `MockCompletionModel` (dev-dep `test-utils`).

From 0.44 (§3.8, ex-S2): constructors for `CompletionRequest`/`Usage`/`Text`; `Message::Assistant(AssistantMessage)`; `ToolFunction.arguments` as Map, `invalid_arguments` answered with an error, `ToolResult.is_error`; Anthropic automatic caching and OpenRouter `anthropic/` caching move to the request's `.cache(CacheRetention::Short)` (and every option set on an agent builder is also set on Hotline's hand-built request: report §7.2); Ollama via `native_completion`; `_` arms on non-exhaustive enums; `Reasoning{text,..}`; decide `accept_unknown_finish_reasons` for voice and record why in docs/design.md.

From 0.45 (ex-S3): `Arc<Message>`/`Arc<ToolDefinition>`; `ToolDefinition::new`; `add_dynamic_tool`; manual bearer header; hooks' `history: &[Arc<Message>]`. Auth, in this same slice or every ChatGPT and Copilot user is signed out: `Authenticator::new(OAuth, LoginDirStore, ..)` where `LoginDirStore` (new `providers/rig_login.rs`) is Hotline's own `TokenStore` over today's login dir: chatgpt → the existing plaintext `auth.json`, copilot → `copilot.json` migrated once from `access-token` + `api-key.json` (old files removed only after the write succeeds); per-credential `lock()` keyed by credential path, never per provider (the Copilot device flow holds it up to 15 minutes); one per-credential authenticator registry shared by turns, voice, image generation and discovery; the signing transport calls `auth.auth_context()`; `refreshed_auth`/`read_session` stop reading files; sign-in via `sign_in_with_device_code` / `auth_context`. docs/security.md applies (credential storage): state default, grant source, enforcement points, tests, residual risk in the PR body and update security.md.

Proof: every `rig_pin_` test from S0 passes with its assertions and expected values unchanged (only fixture construction may change). New tests, prefixed `rig_login_`: a 0.42-layout Copilot dir migrates to `copilot.json` and old files go only after the write; a 0.42 ChatGPT `auth.json` loads unchanged; two clients refreshing at once spend the refresh token once (stub token server). Manual provider checks (one turn each on real sign-ins) are George's before release, not yours: list them in your final note.

## Log
- 2026-10-20T11:59:56.658Z: [claude] George, 2026-10-20 06:59 (call): go straight to 0.45 rather than one version at a time. This task now carries S1, S2 and S3 together (task-232 and task-233 are folded in): every change listed for 0.43, 0.44 and 0.45 in the report's §3 and §6, done once in its 0.45 form, plus S3's LoginDirStore TokenStore and per-credential authenticator registry so ChatGPT and Copilot users stay signed in. rig =0.45.0, rust-version 1.97.1. The three silent traps are mandatory: delete the Anthropic cache fold (every_input_token, cache_beside_input), re-prepend the system prompt every round, and ship the TokenStore in this same change. S0's pinning tests must pass unchanged. For the agent: run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done.
