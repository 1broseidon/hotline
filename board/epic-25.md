---
id: epic-25
title: "Accounts: more than one of any provider, picked per teammate, merged into the model picker; the scenarios in docs/design/accounts-scenarios.html are true"
type: epic
column: todo
position: 2
priority: high
tags:
  - accounts
  - providers
  - picker
  - 0.43.0
createdAt: "2026-10-24T06:22:58.872Z"
---

## Description
George, 2026-10-24 01:12-01:20 (phone). We build the multi-account core ourselves in 0.43; kelvinymoura's draft PR #202 (Claude Code accounts, issue #199) is then redone on top of it for Claude Code, Codex, Grok Build accounts and failover.

Decisions (George, 2026-10-24 01:20, "go with your defaults"):
1. The picker's top level is accounts: one row per account; a provider with one account shows its name, with two or more each row is "Provider · Name". An account opens its models (maker marks, thesvg MIT), a model opens its efforts.
2. The menu head names the current model and account, with effort as a slider (one stop per level the model has), not tabs.
3. The button stays model · effort with the maker's mark; the account is only in the menu head and tooltip.
4. Voice, image generation and chapter notes follow the teammate's account.
5. Switching account waits until the teammate is idle; a revoked or deleted account never falls back to another.

Mock: workspace mockups/picker/index.html (George approved). Acceptance set: mockups/picker/accounts-scenarios.html, to be committed as docs/design/accounts-scenarios.html.

Code map (2026-10-24): every credential already has its own id, label, secret, login folder and model cache (hotline-contract/src/credentials.rs:20, vault.rs). The bottleneck is Vault::connections_from (vault.rs:390-407), keyed by provider with first-created-wins, feeding provider_auth(), connection(provider_id), ModelSources, and the ProviderKeys trait (session.rs:172) used by turns (driver build.rs:228-234), recovery, failure, chapter notes (session/chapters.rs:980), voice (voice/connected.rs:47) and image generation (imagegen/providers.rs:243). Persona stores only provider/model (persona.rs:86). Providers.tsx:93-94 hides connected providers from the add list. vault.rs one_key_per_provider_and_the_first_one_created_wins (:1587) pins first-wins.
