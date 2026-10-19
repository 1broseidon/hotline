---
id: task-8
title: A provider refusing turns shouldn't read as "sign in again" when the sign-in is fine
priority: high
tags:
  - bug
  - bro-126
createdAt: "2026-10-11T21:05:19.683Z"
description: |-
  On 25 Sep 2026 (\~17:30 CT), OpenAI started refusing ChatGPT sign-ins on `chatgpt.com/backend-api/codex/responses` with `401 invalid_api_key` ("Incorrect API key provided… [platform.openai.com/account/api-keys](<http://platform.openai.com/account/api-keys>)"). Every ChatGPT-backed teammate failed. The same token still got a 200 from `/codex/models`, and the official Codex CLI (0.154.0, with its own separate login) failed identically. The cause was on OpenAI's side.

  Hotline showed each failed turn as **"Provider sign-in required: The provider refused this connection. Check its credentials in Settings."** So the person signed out, signed in again, and restarted the desk. None of that could help, and it cost a working login plus a restart.

  ## What to change

  When a turn's request comes back 401/403 (`hotlineFailure.kind = "auth"`), tell these cases apart before telling someone to sign in again:

  * **The sign-in itself is bad.** The token refresh failed, the token is missing or expired, or rig says "ChatGPT sign-in required". Keep today's notice: sign in again under Settings.
  * **The sign-in is good but the provider refused the turn.** The token is current, and the provider's cheap authenticated read (model discovery for ChatGPT/Copilot) still succeeds. Say so instead, for example **"ChatGPT is refusing requests"**, with the summary "Your sign-in still works, so signing in again won't help. Try again later or switch this teammate to another provider." Keep the provider's raw error in details.

  Implementation notes:

  * The check can reuse the discovery request (`providers/chatgpt.rs` `list_models_at`). Run it once per failure, not per retry, and cache the result briefly so a run of failures doesn't hammer the provider.
  * Don't mark the credential as needing attention in Settings in the second case.
  * Apply the same logic to any provider with a discovery endpoint. Otherwise fall back to today's wording.

  ## Acceptance

  * Reproduce by pointing a ChatGPT login at a stub that returns 200 on `/models` and 401 `invalid_api_key` on `/responses`. The notice should say the provider is refusing and must not tell the person to sign in again.
  * A genuinely revoked token still gets "Provider sign-in required".

  Migrated from Linear BRO-126: https://linear.app/1broseidon/issue/BRO-126/a-provider-refusing-turns-shouldnt-read-as-sign-in-again-when-the-sign (2026-10-11).

  0.40 triage (2026-10-18): In scope as is: the wording plus a one-off discovery probe. driver/failure.rs:273-275 still says 'Provider sign-in required… Check its credentials' for every Kind::Auth. Probe providers/chatgpt.rs list_models_at once per failure, cache the verdict briefly, and say 'ChatGPT is refusing requests' (or the provider's name) when the sign-in still lists models.
updatedAt: "2026-10-19T00:07:33.218Z"
parentId: epic-14
relatedFiles:
  - crates/hotline-core/src/driver/failure.rs
  - crates/hotline-core/src/providers/chatgpt.rs
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/src/driver/failure.rs
      description: a stub that lists models but refuses /responses with 401 reads as the provider refusing, not sign in again; a revoked token still reads Provider sign-in required
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  constraints:
    - Probe once per failure, not per retry, and cache the verdict briefly
    - Do not mark the credential as needing attention when the sign-in still lists models
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-18T23:54:29.055Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:05:28.131Z"
    duration: 659
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-18T23:54:14.359Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-18T23:54:15.800Z"
    ended: "2026-10-19T00:05:35.770Z"
    outcome: review
assignee: claude
completedAt: "2026-10-19T00:07:33.218Z"
---

## Log
- 2026-10-19T00:05:31.884Z: [claude] Done on branch dispatch/task-8 (c3d30f8c). Before a turn's notice goes out, a 401/403 (Kind::Auth) on a ChatGPT, Copilot or xAI-subscription sign-in runs Failure::checked_against_sign_in (driver/rig.rs). That asks the provider's model list once, within a 10s bound, and caches the verdict per provider for 60s. If the list answers, the failure becomes the new Kind::ProviderRefused: '<Provider> is refusing requests', 'Your sign-in still works, so signing in again won't help…', raw error kept in details. Otherwise it stays 'Provider sign-in required'. API keys and other providers keep today's wording. Nothing marks the credential in Settings. The UI offers 'Run again' for provider_refused. For later tasks: Kind serializes as "provider_refused", and chatgpt::list_models_at is now pub(crate) so tests can point it at a stub. Tests in driver/failure.rs: a stub that lists models but answers /responses with 401 vs a revoked token, one probe for a run of failures, and only sign-ins get probed. fmt, clippy, the full hotline-core tests and the contract.ts diff all pass.
- 2026-10-19T00:07:32.707Z: Landed on release/0.40.0
