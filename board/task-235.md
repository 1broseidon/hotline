---
id: task-235
title: "S5: sign-out revokes at the provider; a revoked login stops serving at once"
column: backlog
position: 44
priority: low
tags:
  - rig
  - auth
assignee: claude
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
updatedAt: "2026-10-20T12:03:24.043Z"
---

## Description
Report §6 S5 (docs/plans/rig-0.45-migration.md). WAITS ON GEORGE: the epic leaves 'revoke tokens at the provider on credential delete' open for him; the lead does not hand this off until George answers on epic-22. If he agrees: ChatGPT `sign_out(&http)` best effort on delete; registry eviction plus `forget()` on revoke/delete; a Copilot guard (`CredentialSource` wrapper) that checks the login is still live before each request. Tests, prefixed `rig_revoke_`, with a stub revoke endpoint: delete calls revoke once and still deletes when revoke fails; a revoked Copilot login stops serving within one request. docs/security.md applies. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).
