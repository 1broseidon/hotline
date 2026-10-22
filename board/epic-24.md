---
id: epic-24
title: "Credentials: an agent asks for a login in the chat, keeps it in our vault, and uses each key only as you allow; the scenarios in docs/design/credential-scenarios.html are true"
type: epic
column: todo
position: 0
priority: high
tags:
  - credentials
  - computer
  - phone
  - 0.43.0
createdAt: "2026-10-22T03:08:32.403Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T04:35:32.443Z"
---

## Description
George, 2026-10-21 (21:55–22:04). Three ideas, one epic:

1. Capture in the chat. When a teammate reaches a site it has no key for, it raises a credential card in the chat, on the desk and on the phone. The person types the login into secure fields there. The values go straight into the vault (OS keychain, crates/hotline-core/src/vault/shared.rs), are granted to that teammate for that site, and are delivered to its computer. They never reach the agent, the transcript, the tape or the phone's cache. The teammate then signs in with browser fill secret NAME.password, and the computer's extension card shows the new key.
2. Approval per key. Each login and passkey is either automatic or ask me. An ask-me key is held by the computer when a site asks for it or the agent fills it, until the person approves on the desk or the phone (with an allow-for-10-minutes choice). The extension card on the page says it is waiting. Low-stakes sites stay automatic.
3. OS vaults, agnostically. Instead of a connector per vault (1Password, Bitwarden, KeePass), ask the OS: macOS and Windows 11 route passkey requests to whichever provider the person uses. A passkey kept there never leaves it, so every use is signed on the host, and the OS prompt (Touch ID, Windows Hello) is the approval. Passwords have no OS-wide equivalent and Linux has no passkey provider API yet; for those, capture (1) is the path. This depends on Apple granting the web-browser passkey entitlement, so it starts as a spike.

History: hotline-mobile epic-1 (answer a computer credential prompt from the phone) was closed won't-do on 2026-10-11: a live typing channel into the computer wasn't worth its surface. This differs: values go into the vault through one command, and the computer only ever gets them through the existing PUT /secrets.

Rules, carried from earlier decisions:
- Credentials are never asked or answered by voice (voice epic-21: cards by voice are approvals and simple questions only). A credential card on a call thread shows on the desk and phone only.
- A card's value fields are never a note: nothing typed in them is sent to the agent or written to a tape.
- An agent learns a key's name and kind, never its value (session/preamble.rs, describe_secret).

What exists (origin/main): vault StoredSecret {Variable, Login, Passkey}, grants in PersonaComputer.secrets, delivery in computer/secrets.rs deliver() via Room::hand_secrets, passkey arming and its approval card (session/secrets.rs, PasskeyAskCard in ui/src/components/transcript/Asks.tsx), human cards (session/asking.rs, HumanAction), phone seat permissions (wire/seat.rs:84–134). No agent tool for secrets yet. Side note found while mapping: hand_secrets' doc says it reports missing names on the tape, but it only writes to stderr.

The computer's half (extension, holding a request for approval) lives in ~/Projects/personal/hotline-computer, which has no board, so its tasks sit here tagged hotline-computer. Phone tasks are on the hotline-mobile board and name this epic.

Order:
1. The scenario deck first; George walks it; its must-be-true lines become each story's Done when.
2. Capture card on the desk, then the phone, with the extension showing the new key.
3. Approval policy: desk, computer, phone.
4. The OS-vault spike; the OS-vault passkeys only if it says yes.

## Log
- 2026-10-22T03:09:24.551Z: [claude] Phone tasks on the hotline-mobile board: task-49 (capture card), task-50 (approval card). Computer task here: task-268 (tagged hotline-computer, since that repo has no board).
- 2026-10-22T04:35:32.931Z: [claude] Decisions (George, 2026-10-21 23:32: "Agree with your leans"):
1. The agent proposes the key's name; the person can edit it on the card.
2. When the person already has a key for that site, the card offers "Grant NAME instead".
3. Ask me is off by default: a new key is Automatic until the person flips it.
4. A credential card expires after 30 minutes.
5. After a decline, the teammate does not ask again for that site in the same turn.
6. Make a passkey keeps the site's own approval card: nothing is made without Approve.
7. Allow 10 min covers one key, one site, one teammate.
8. On the phone, Approve and Allow 10 min need Face ID.
9. The approval card shows the page's path, not just the site.
10. No built-in list of sensitive sites; the agent may suggest Ask me.
11. Nothing about credentials is answered by voice, ask-me approvals included. If the person starts saying a password on a call, the voice cuts in and points to the card.
12. OS vault: no phone fallback when the Mac is away; the passkey card offers "Keep in your passwords app" as a second choice; macOS before Windows.

Acceptance set committed: docs/design/credential-scenarios.html on release/0.43.0 (e8f68de8). Each story's Done when now carries the deck's must-be-true lines. Lands on release/0.43.0.
- 2026-10-22T07:54:30.924Z: [claude] Overnight 2026-10-22: task-265 deck done; task-266 (desk credential card) merged via PR #200 with CI green; task-267 (automatic / ask me, approval card, use door client) landed dbb4d279, make check green (2,278 tests); task-268 (computer hold + card states) built on hotline-computer ext/v3 as rc.7, contract test green, in Review; task-269 spike in Review (macOS needs Apple's browser entitlement; Windows works today); hotline-mobile task-49 and task-50 landed on release/0.11.0 (1,022 tests). Not done: a desk-to-computer end-to-end run (desk 0.43 dev build with an rc.7 computer); task-270 waits on the spike decision.
- 2026-10-22T15:04:02.631Z: [claude] George's end-to-end check, 2026-10-22 10:03, dev desk 0.43 (80e73e9b) with hotline-computer:0.11.0-rc.7: saving a login from the credential card works; Decline works; an Ask me login raised the approval card and Allow 10 min worked. Recorded as the manual check for task-266, task-267 and task-268.
