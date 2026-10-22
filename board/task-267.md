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
updatedAt: "2026-10-22T04:35:54.750Z"
---

## Description
Each login and passkey is Automatic or Ask me. StoredSecret's sidecar (vault/shared.rs About) and the contract's SharedSecret gain use: Auto|Ask; keys stored before this are Automatic. Settings → Secrets (ui/src/components/Secrets.tsx) and the capture card set it. It rides to the computer in wire_form (computer/secrets.rs). For a held use the desk raises one approval card, SecretUseAsk {secret, kind, site, path, teammate}, on desk and phone (hotline-mobile task-50), with Approve, Allow 10 min (one key, one site, one teammate) and Deny, and carries the answer to the computer's door (task-268). No built-in list of sensitive sites. Not answered by voice.

Acceptance set: docs/design/credential-scenarios.html, scenes 5–6. Decisions: epic-24 note of 2026-10-21.

Done when:
- Approved, the passkey is stored under the card's name, granted to the teammate, with the card's Ask me setting. (named test; scene: The site wants a passkey instead)
- An ask-me key's use raises one approval card, on the desk and the phone, and its answer reaches the computer. (named test against the computer's fake door; scene: An ask-me key on github.com)
- Allow 10 min lets later uses of that key on that site through with no card until it lapses. (named test; scene: An ask-me key on github.com)
- Deny tells the agent plainly that the person declined. (named test; scene: An ask-me key on github.com)
- If the page stops waiting first, the card expires on both seats and the agent hears that it lapsed. (named test; scene: An ask-me key on github.com)
- An automatic key's use raises no card and is not held. (named test; scene: An automatic key on a low-stakes site)
- Settings → Secrets shows each key as Automatic or Ask me and changes it; the choice survives restart. (UI test plus vault test; scene: An automatic key on a low-stakes site)
- Keys stored before this change are Automatic. (vault migration test; scene: An automatic key on a low-stakes site)
- The setting rides to the computer with the set (wire_form); a change reaches it without a restart and applies from the next use. (named test; scene: An automatic key on a low-stakes site)
- make check passes.
