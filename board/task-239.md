---
id: task-239
title: "Cards on a call: what needs the person is said on the call, and approvals and simple questions are answered by voice"
column: in-progress
position: 1
priority: high
tags:
  - voice
  - security
parentId: epic-21
createdAt: "2026-10-20T20:13:08.678Z"
dependsOn:
  - task-225
updatedAt: "2026-10-21T07:08:22.021Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T06:53:08.339Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T06:53:11.736Z"
assignee: codex
decisions:
  - decision-15
---

## Description
Scenario: Something needs you during the call. Decision (George accepted the deck's lean, 2026-10-20): approvals and simple questions may be answered by voice, and nothing that needs a credential, a 2FA tap, a screen, a payment or a deletion may.

Done when:
- A card that would wake the person in the chat is said on the call instead, once, at a natural break, not over the person.
- An approval or a simple question is answered by a yes or no on the call, and the answer is written to the chat as a record, from the call.
- A card needing a credential, a 2FA tap, a screen, a payment or a deletion is said and never answered by voice. It waits for the desk.
- The card handler enforces which cards may be answered by voice, never the model, as a capability change under docs/security.md. It's proved allowed and refused through the real handler.

## Log
- 2026-10-21T07:00:49.571Z: [codex] Implementing a stable-ID call card queue and core-only answer path. Existing action/passkey/opaque driver cards fail closed at the desk; core-owned collaboration approvals allow only a live session. Adding a separate informational yes/no question path with no operation authority, lease checks, and from-call records; validating through real room/wire handlers.
