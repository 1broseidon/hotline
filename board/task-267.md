---
id: task-267
title: "Desk: each login and passkey is automatic or ask me, with an approval card for ask-me uses"
column: todo
position: 3
priority: high
tags:
  - credentials
  - policy
parentId: epic-24
createdAt: "2026-10-22T03:09:08.664Z"
---

## Description
StoredSecret's sidecar and the contract's SharedSecret gain use: Auto|Ask (default Auto for what exists, Ask chosen per key in Settings → Secrets and on the capture card). The flag rides to the computer in wire_form. The desk polls (or is told by) the computer for held uses, raises an approval card (SecretUseAsk {secret, site, teammate}) on desk and phone, and answers with approve once, allow for 10 minutes, or deny.

Done when:
- Settings shows and changes the setting; it survives restart: UI test plus vault test.
- An ask-me key's use raises one card and its answer reaches the computer: named test against the computer's fake door (computer/guide.rs fakes).
- Allow for 10 minutes lets later uses of that key on that site through without a card until it lapses: named test.
- Deny tells the agent plainly that the person declined: named test.
