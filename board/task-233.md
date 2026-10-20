---
id: task-233
title: "S3: Rig 0.45 with Hotline's own TokenStore over today's login dirs (no one signed out)"
column: todo
position: 5
priority: high
tags:
  - rig
  - auth
  - security
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:44.208Z"
dependsOn:
  - task-232
updatedAt: "2026-10-20T06:51:47.584Z"
---

## Description
Report §6 S3 and §4. rig =0.45.0, rust-version 1.97.1; Arc<Message>/Arc<ToolDefinition>; ToolDefinition::new; add_dynamic_tool; manual bearer header. Auth: Authenticator::new(OAuth, LoginDirStore, ..) with a per-credential authenticator registry and per-credential lock(); LoginDirStore maps chatgpt → the existing auth.json and copilot → copilot.json migrated once from access-token + api-key.json (old files removed only after the write); the signing transport calls auth.auth_context(); refreshed_auth/read_session stop reading files; sign-in via sign_in_with_device_code / auth_context. Tests: a 0.42-layout Copilot dir migrates; a ChatGPT auth.json loads unchanged; two clients refreshing at once spend the refresh token once. Manual: a profile signed in under 0.42 keeps working with no re-sign-in. security.md applies (credential storage).

## Log
- 2026-10-20T11:59:58.047Z: [claude] Folded into task-231 (George chose the direct jump to 0.45, 2026-10-20 06:59).
