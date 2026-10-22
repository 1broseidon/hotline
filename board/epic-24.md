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
createdAt: "2026-10-22T03:08:32.403Z"
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
