---
id: task-234
title: "S4: ChatGPT and Copilot tokens move into the vault (keychain)"
column: todo
position: 6
priority: medium
tags:
  - rig
  - auth
  - security
assignee: claude
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
updatedAt: "2026-10-20T12:03:22.450Z"
---

## Description
Report §6 S4 and §4.3 (docs/plans/rig-0.45-migration.md). Builds on S1's `LoginDirStore` in `providers/rig_login.rs`. Its body becomes `CredentialFile` (keychain) under `spawn_blocking`: ChatGPT's plaintext `auth.json` migrates to a reference file on first `read()`; `copilot.json` is written through `CredentialFile::write`; `begin_login` stops pre-creating Rig files; `abandon_login` deletes the `copilot.json` credential; `AuthError::Store` maps (in `driver/failure.rs`) to 'unlock your keychain', not 'sign in again'. Update vault.rs's header comment (lines 1-2) and docs/security.md (credential storage; PR body states default, grant source, enforcement points, tests, residual risk). Release note in docs/releases for 0.42.0: a downgrade to an older Hotline shows sign-in required.

Tests, prefixed `rig_vault_`, on the `MemoryStore` backend: plaintext auth.json → reference swap; a keychain failure keeps the plaintext and returns the 'unlock your keychain' error; delete removes every chunk; a 0.42-layout Copilot dir migrates straight into the keychain. Manual macOS keychain check is George's before release: list it in your final note. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).
