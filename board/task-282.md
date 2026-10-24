---
id: task-282
title: "Core: an account is a credential; model choices and teammates name their account, and every turn runs on it"
column: todo
position: 3
priority: high
tags:
  - accounts
  - vault
  - core
parentId: epic-25
createdAt: "2026-10-24T06:23:15.523Z"
---

## Description
S3, S7, S9 of docs/design/accounts-scenarios.html.

Today Vault::connections_from (vault.rs:390-407) keeps one credential per provider, first created wins, and everything resolves by provider id.

Done when:
- A model choice carries its account (credential id, name, kind); two accounts of one provider give two sets of choices with distinct ids. (models.rs test)
- A teammate stores its account with its model; turns, retries and recovery resolve the credential by that account, never by provider. (driver harness test: two Anthropic keys, two teammates, each request carries its own key)
- The account a turn ran on is recorded with the turn. (session test)
- Existing rooms: a teammate with no account runs on the first-created credential of its provider, exactly as 0.42; room defaultModelId/lastModelId resolve the same way. (migration test)
- one_key_per_provider_and_the_first_one_created_wins is replaced by tests of the new rule, not deleted silently.
- An account kind exists for a CLI login folder, unused in 0.43, so harness accounts are just another account (S9).
- docs/security.md describes accounts: what one is, where each kind's secret lives, and that turns never fall back.
