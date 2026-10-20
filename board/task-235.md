---
id: task-235
title: "S5: sign-out revokes at the provider; a revoked login stops serving at once"
column: review
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
updatedAt: "2026-10-20T15:07:19.291Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-20T14:46:38.518Z"
    reworkCount: 0
    deliveredAt: "2026-10-20T15:07:19.291Z"
    duration: 1241
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T14:46:15.176Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T14:46:17.245Z"
---

## Description
Report §6 S5 (docs/plans/rig-0.45-migration.md). WAITS ON GEORGE: the epic leaves 'revoke tokens at the provider on credential delete' open for him; the lead does not hand this off until George answers on epic-22. If he agrees: ChatGPT `sign_out(&http)` best effort on delete; registry eviction plus `forget()` on revoke/delete; a Copilot guard (`CredentialSource` wrapper) that checks the login is still live before each request. Tests, prefixed `rig_revoke_`, with a stub revoke endpoint: delete calls revoke once and still deletes when revoke fails; a revoked Copilot login stops serving within one request. docs/security.md applies. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).

What S1 (task-231) left you: the authenticator registry (`rig_login::chatgpt(dir)`/`copilot(dir)`, keyed by login dir, shared by turns, voice, image generation, discovery and the failure probe) never evicts, so a revoked or deleted login's in-memory token lives until expiry, though nothing asks for it. Eviction on delete belongs here even if George declines provider-side revoke. Requests are signed in `providers/outgoing.rs` `Outgoing` via `auth_context`, the natural place for the Copilot liveness guard.

## Log
- 2026-10-20T14:46:14.257Z: [claude] George, 2026-10-20 09:46: yes for ChatGPT — revoke at the provider when a credential is deleted, best effort (Rig 0.45 chatgpt::auth sign_out(&http); a failed revoke is ignored and the local delete still happens). Copilot has no revoke API for this sign-in: offer a 'Revoke at GitHub' link to the user's GitHub applications settings instead. Plus the parts that need no decision: evict the per-credential authenticator registry entry and forget() on revoke/delete; a Copilot guard that checks the login is still live before each request. security.md applies. For the agent: run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done.
- 2026-10-20T14:48:56.044Z: [codex] Picked up contract. Rig 0.45 ChatGPT sign_out revokes each stored refresh/access token once; Copilot has no forget() API. Implementing one guarded registry for both: ChatGPT forget plus eviction, Copilot drops its authenticator, with liveness checked before/after token resolution. Async delete will serialize per login, revoke locally first, attempt bounded provider sign-out, and delete despite provider failure.
- 2026-10-20T14:54:29.893Z: [codex] Implemented guarded CredentialSource shared by all login consumers, registry eviction/cache release on revoke/delete/abandon, async serialized ChatGPT delete with bounded best-effort sign_out, and working Revoke at GitHub action. Added retained-client, refresh-overlap, and real-wire revoke endpoint regressions; security.md updated, migration report untouched. Foreground make check is running.
- 2026-10-20T15:02:37.748Z: [codex] make check passed (UI, dependencies, Python, fmt, clippy and full workspace tests); all four rig_revoke_ tests pass. Local record operations now serialize with revoke/delete to prevent a late refresh from recreating deleted storage. Committing, then performing the requested release merge and rerunning make check and precheck.
- 2026-10-20T15:03:32.957Z: [codex] Committed 3bb68e93 on dispatch/task-235. Requested git merge release/0.42.0 reports already up to date. Post-merge make check is running; precheck follows. No wire types or migration-report edits.
- 2026-10-20T15:05:23.388Z: [codex] Post-merge make check passed. Precheck has passed fmt, lockfile, workflow lint, security citations, generated contract and clippy; Windows compilation is running, then isolated UI tests. Both required contract.ts comparisons remain empty.
- 2026-10-20T15:07:19.020Z: [codex] Delivered implementation on dispatch/task-235, commit 3bb68e93; clean worktree. ChatGPT delete locally revokes first, calls Rig sign_out best effort with a ten-second bound, and still deletes on provider failure; each stored token is revoked once. Guarded CredentialSource covers existing S1 consumers, evicts and forgets ChatGPT/drops Copilot caches on revoke/delete/abandon, checks liveness before/after refresh, and serializes local token writes with removal. Copilot offers Revoke at GitHub. Default/grant source: existing device sign-ins and owner-only credential commands, no new authority. Enforcement: real desk/wire handler, guarded signing transport and token store. Tests: four rig_revoke_ tests (retained transport, fresh lookup/cache invalidation, refresh overlap, successful/failing stub revoke via wire); make check passed before and after requested release/0.42.0 merge (already up to date), make precheck all eight legs passed, shipping core check passed, both contract.ts comparisons empty. Residual risk: already-dispatched requests may finish, failed/timed-out provider revoke can leave tokens usable outside Hotline, and Copilot provider access requires manual GitHub revocation. No wiring left outstanding. Migration report intentionally untouched; reviewer can merge, closing task updates it.
