---
id: task-234
title: "S4: ChatGPT and Copilot tokens move into the vault (keychain)"
column: in-progress
position: 6
priority: medium
tags:
  - rig
  - auth
  - security
assignee: codex
relatedFiles:
  - crates/hotline-core/src/providers/rig_login.rs
  - crates/hotline-core/src/vault.rs
  - crates/hotline-core/src/driver/failure.rs
  - docs/security.md
  - docs/releases
parentId: epic-22
createdAt: "2026-10-20T06:51:44.693Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T13:37:36.877Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-core/src/providers/rig_login.rs
      description: LoginDirStore backed by CredentialFile (keychain)
    - type: test
      path: crates/hotline-core
      description: rig_vault_* tests on MemoryStore
    - type: docs
      path: docs/security.md
      description: ChatGPT and Copilot tokens in the vault
  validation:
    commands:
      - make check
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_vault_')" -ge 4
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_pin_')" -ge 6
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - A keychain failure never deletes or overwrites the plaintext credential
    - Do not edit docs/plans/rig-0.45-migration.md
  metrics:
    readyAt: "2026-10-20T12:03:22.704Z"
    pickedUpAt: "2026-10-20T13:37:36.877Z"
    reworkCount: 1
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T13:37:08.139Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T13:37:09.319Z"
---

## Description
Report §6 S4 and §4.3 (docs/plans/rig-0.45-migration.md). Builds on S1's `LoginDirStore` in `providers/rig_login.rs`. Its body becomes `CredentialFile` (keychain) under `spawn_blocking`: ChatGPT's plaintext `auth.json` migrates to a reference file on first `read()`; `copilot.json` is written through `CredentialFile::write`; `begin_login` stops pre-creating Rig files; `abandon_login` deletes the `copilot.json` credential; `AuthError::Store` maps (in `driver/failure.rs`) to 'unlock your keychain', not 'sign in again'. Update vault.rs's header comment (lines 1-2) and docs/security.md (credential storage; PR body states default, grant source, enforcement points, tests, residual risk). Release note in docs/releases for 0.42.0: a downgrade to an older Hotline shows sign-in required.

Tests, prefixed `rig_vault_`, on the `MemoryStore` backend: plaintext auth.json → reference swap; a keychain failure keeps the plaintext and returns the 'unlock your keychain' error; delete removes every chunk; a 0.42-layout Copilot dir migrates straight into the keychain. Manual macOS keychain check is George's before release: list it in your final note. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).

What S1 (task-231) left you, as landed on release/0.42.0: `LoginDirStore` reads chatgpt's `auth.json` as-is and copilot's `copilot.json`, migrating lazily on first load from `access-token` + `api-key.json` (old files removed only after the write) — your keychain migration must take both the 0.42 layout and S1's plaintext `copilot.json`. A tokio lock per record file; the registry `rig_login::chatgpt(dir)`/`copilot(dir)` is keyed by login dir and shared by turns, voice, image generation, discovery and the failure probe. `ProviderAuth::Login { token_dir }` is unchanged. Today `begin_login` pre-creates `copilot.json` as `{}` and the vault test `a_copilot_login_precreates_the_record_rig_will_write` pins that: replace that test with the new behaviour. Copilot sign-in uses a separate allow-device-flow authenticator over the same store and lock; ChatGPT sign-in uses the registry clone + `sign_in_with_device_code`. Do not touch providers/outgoing.rs, providers/copilot.rs, providers/compat.rs, driver/rig/build.rs or turn.rs (S7 runs alongside), nor voice/dispatcher.rs (S6).

## Log
- 2026-10-20T13:37:05.469Z: [claude] George, 2026-10-20 08:35: the rest of epic-22 is mechanical and goes to Codex on gpt-6-astra. A Claude Opus run started this and was stopped after about 5 minutes; its worktree may hold partial uncommitted edits: review them, keep what's right, discard the rest. Run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done (a landing's check stops after 15 minutes).
- 2026-10-20T13:37:48.287Z: [codex] Picked up the contract and reviewed inherited edits: vault backend registration, CredentialFile under spawn_blocking, removal of pre-created login records, and keychain failure classification. Finishing coverage and docs before foreground make check, release merge, and recheck.
- 2026-10-20T13:43:15.843Z: [codex] All 8 rig_vault_* tests pass on MemoryStore, including partial-write failure preserving plaintext, both Copilot migrations, multi-chunk delete/abandon, and credential.delete over the real wire. Keeping inherited backend registry so --store file is honored without changing ProviderAuth::Login. make check running in foreground; docs/design.md, docs/log.md and security docs now match storage behavior; migration plan untouched.
