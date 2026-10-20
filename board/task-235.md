---
id: task-235
title: "S5: sign-out revokes at the provider; a revoked login stops serving at once"
column: todo
position: 7
priority: low
tags:
  - rig
  - auth
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
parentId: epic-22
createdAt: "2026-10-20T06:51:45.174Z"
dependsOn:
  - task-234
updatedAt: "2026-10-20T06:51:48.535Z"
---

## Description
Report §6 S5. ChatGPT sign_out(&http) best effort on delete (if George agrees); registry eviction plus forget() on revoke/delete; a Copilot guard that checks the login is still live before each request.
