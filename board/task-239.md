---
id: task-239
title: "Cards on a call: what needs the person is said on the call, and approvals and simple questions are answered by voice"
column: backlog
position: 44
priority: high
tags:
  - voice
  - security
parentId: epic-21
createdAt: "2026-10-20T20:13:08.678Z"
dependsOn:
  - task-225
updatedAt: "2026-10-20T20:13:24.662Z"
---

## Description
Scenario: Something needs you during the call. Decision (George accepted the deck's lean, 2026-10-20): approvals and simple questions may be answered by voice, and nothing that needs a credential, a 2FA tap, a screen, a payment or a deletion may.

Done when:
- A card that would wake the person in the chat is said on the call instead, once, at a natural break, not over the person.
- An approval or a simple question is answered by a yes or no on the call, and the answer is written to the chat as a record, from the call.
- A card needing a credential, a 2FA tap, a screen, a payment or a deletion is said and never answered by voice. It waits for the desk.
- The card handler enforces which cards may be answered by voice, never the model, as a capability change under docs/security.md. It's proved allowed and refused through the real handler.
