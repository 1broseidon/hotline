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
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:44.693Z"
---

## Description
Report §6 S4 and §4.3. LoginDirStore's body becomes CredentialFile under spawn_blocking: ChatGPT's auth.json migrates on first read; copilot.json is written through CredentialFile::write; begin_login stops pre-creating Rig files; abandon_login deletes the copilot.json credential; AuthError::Store maps to 'unlock your keychain', not 'sign in again'. Update vault.rs's header comment. Release note: a downgrade to an older Hotline shows sign-in required.
