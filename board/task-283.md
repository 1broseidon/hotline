---
id: task-283
title: Switching a teammate's account waits for idle; a revoked or deleted account never falls back
column: todo
position: 4
priority: high
tags:
  - accounts
  - core
  - session
parentId: epic-25
createdAt: "2026-10-24T06:23:16.091Z"
---

## Description
S4, S5.

Done when:
- Moving a mid-turn teammate to another account lets the turn finish on the old one and starts the next on the new one, chapters carrying the conversation. (session harness test)
- A teammate whose account is revoked or deleted fails its next turn with a card naming the account and offering the other accounts that serve its model; nothing runs on another account unless the person picks it. (harness test)
- Picking an account on that card moves the teammate and retries. (harness test)
- The picker shows the new account as pending until the switch lands. (UI test)
