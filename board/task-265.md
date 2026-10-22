---
id: task-265
title: "Credential scenarios deck: the acceptance set for epic-24"
column: todo
position: 1
priority: high
tags:
  - credentials
  - design
parentId: epic-24
createdAt: "2026-10-22T03:09:07.586Z"
---

## Description
A folio deck at docs/design/credential-scenarios.html, built like docs/design/voice-scenarios.html, for George to walk through before any code.

Scenes, each with what the person sees (desk, phone, the computer's extension card) and a 'What must be true' list:
1. A teammate reaches a sign-in page it has no key for and raises a credential card; George fills it on the desk.
2. The same, answered on the phone while away.
3. Declined, and expired: the teammate is told and stops; nothing is stored.
4. The site wants a passkey instead: the card offers Make a passkey, which runs the existing arming and approval.
5. An ask-me key (github.com passkey): the agent clicks Sign in with a passkey, the computer holds it, the approval card shows on desk and phone, the extension says it is waiting; approve, allow for 10 minutes, deny.
6. An automatic key on a low-stakes site: it just goes through, with the note on the page.
7. On a call: the credential card appears on the desk and phone, and the voice says only that it is waiting there.
8. OS vault (if the spike allows): a passkey kept in the person's own provider; Touch ID on the Mac signs each use.

Done when:
- The deck is committed at docs/design/credential-scenarios.html.
- George has walked it and his decisions are recorded on epic-24.
- Each story below carries the deck's must-be-true lines as its Done when.

## Log
- 2026-10-22T04:35:56.776Z: [claude] Deck built and walked: George agreed with every lean (2026-10-21 23:32). Committed to release/0.43.0 as docs/design/credential-scenarios.html (e8f68de8); its must-be-true lines are now the Done when of task-266 to task-270 and hotline-mobile task-49 and task-50.
