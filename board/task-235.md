---
id: task-235
title: "S5: sign-out revokes at the provider; a revoked login stops serving at once"
column: todo
position: 2
priority: low
tags:
  - rig
  - auth
assignee: codex
relatedFiles:
  - crates/hotline-core/src/vault.rs
  - crates/hotline-core/src/desk.rs
  - crates/hotline-core/src/providers/rig_login.rs
  - crates/hotline-core/src/providers/copilot.rs
  - docs/security.md
parentId: epic-22
createdAt: "2026-10-20T06:51:45.174Z"
dependsOn:
  - task-234
  - task-237
updatedAt: "2026-10-20T14:46:15.308Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core
      description: rig_revoke_* tests with a stub revoke endpoint
  validation:
    commands:
      - make check
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_revoke_')" -ge 2
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - Delete always removes the local credential, even when provider revoke fails
  metrics:
    readyAt: "2026-10-20T12:03:24.303Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T14:46:15.176Z"
  until: done
  tries: 0
---

## Description
Report §6 S5 (docs/plans/rig-0.45-migration.md). WAITS ON GEORGE: the epic leaves 'revoke tokens at the provider on credential delete' open for him; the lead does not hand this off until George answers on epic-22. If he agrees: ChatGPT `sign_out(&http)` best effort on delete; registry eviction plus `forget()` on revoke/delete; a Copilot guard (`CredentialSource` wrapper) that checks the login is still live before each request. Tests, prefixed `rig_revoke_`, with a stub revoke endpoint: delete calls revoke once and still deletes when revoke fails; a revoked Copilot login stops serving within one request. docs/security.md applies. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).

What S1 (task-231) left you: the authenticator registry (`rig_login::chatgpt(dir)`/`copilot(dir)`, keyed by login dir, shared by turns, voice, image generation, discovery and the failure probe) never evicts, so a revoked or deleted login's in-memory token lives until expiry, though nothing asks for it. Eviction on delete belongs here even if George declines provider-side revoke. Requests are signed in `providers/outgoing.rs` `Outgoing` via `auth_context`, the natural place for the Copilot liveness guard.

## Log
- 2026-10-20T14:46:14.257Z: [claude] George, 2026-10-20 09:46: yes for ChatGPT — revoke at the provider when a credential is deleted, best effort (Rig 0.45 chatgpt::auth sign_out(&http); a failed revoke is ignored and the local delete still happens). Copilot has no revoke API for this sign-in: offer a 'Revoke at GitHub' link to the user's GitHub applications settings instead. Plus the parts that need no decision: evict the per-credential authenticator registry entry and forget() on revoke/delete; a Copilot guard that checks the login is still live before each request. security.md applies. For the agent: run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done.
