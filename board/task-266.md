---
id: task-266
title: "Desk: request_credential tool and an in-chat credential card whose values go only to the vault"
column: in-progress
position: 0
priority: high
tags:
  - credentials
parentId: epic-24
createdAt: "2026-10-22T03:09:08.125Z"
updatedAt: "2026-10-22T05:06:41.883Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T05:06:15.521Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 24193
    at: "2026-10-22T05:06:18.269Z"
assignee: codex
---

## Description
A teammate tool request_credential {site, reason, name?, kind: login|passkey} raises a credential card in the chat, on the desk (and the phone, hotline-mobile task-49). The card shows the full origin, the reason, the proposed name (editable), an Ask me toggle (off by default) and secure fields: username, password, optional one-time-code seed. If the person already has a key for that site, it offers Grant NAME instead. Submit sends one command, secrets.capture.answer {action_id, name, values, ask}, from the owner or phone seat (wire/seat.rs). The desk stores a Login {sites: [origin], username, password, totp?} (vault/shared.rs set_shared), ticks the grant for the asking teammate (PersonaComputer.secrets), re-sends the set (Room::secrets_changed → hand_secrets → computer/secrets.rs deliver), and answers the agent with the name, site and how to fill it. kind passkey has no value fields: Make a passkey runs the existing arming (secrets_passkey_register) and its approval card.

Contract: TranscriptEvent::CredentialAsk {id, site, reason, kind, name, status, stored_as?}, status Pending|Stored|Declined|Expired|Withdrawn; Waiting::Credential for pushes; a ThreadAnswer variant for thread answers. The card expires after 30 minutes. After a decline the teammate does not ask again for that site in the same turn. Never by voice (epic-24 decision 11).

Acceptance set: docs/design/credential-scenarios.html, scenes 1–4 and 7. Decisions: epic-24 note of 2026-10-21.

Done when:
- Save sends one command, secrets.capture.answer, and the desk stores a Login for https://github.com under the chosen name, ticks the grant for Pork Chop and re-sends the set to its computer. (named test: capture stores, grants and delivers; scene: A login, asked for in the chat)
- No typed value appears in the tape, the transcript JSON, the agent's answer or the desk's logs. (named test: capture a known value, grep every sink; scene: A login, asked for in the chat)
- The agent's answer names the key, its site and how to fill it; never a value. (named test; scene: A login, asked for in the chat)
- The password fields are cleared on Save and on dismiss. The stored card shows name, site and teammate only. (UI test in Asks; scene: A login, asked for in the chat)
- The login is bound to the origin the teammate asked for, shown in full on the card, and fills nowhere else. (named test; scene: A login, asked for in the chat)
- The tape holds the ask and its outcome (Stored as GITHUB_LOGIN), nothing more. (same sink test; scene: A login, asked for in the chat)
- The phone seat is allowed secrets.capture.answer (wire/seat.rs); any other phone command carrying values is refused. (seat permission test; scene: Answered on the phone, while away)
- Answered on one seat, the card settles on the other and cannot be answered twice. (named tests; scene: Answered on the phone, while away)
- Decline answers the agent that the person declined; nothing is stored, granted or delivered. (named test; scene: Declined, expired or withdrawn)
- Expiry answers the agent that no answer came; nothing is stored. (named test; scene: Declined, expired or withdrawn)
- Anything typed before a decline, expiry or withdrawal is cleared and never sent. (UI tests; scene: Declined, expired or withdrawn)
- A settled card (Declined, Expired, Withdrawn) shows its outcome on desk and phone and has no Save or Decline. (named tests; scene: Declined, expired or withdrawn)
- The teammate's answer says plainly why it stopped; it does not raise a second card for the same site in the same turn. (named test on the tool; scene: Declined, expired or withdrawn)
- A passkey card has no value fields. Make a passkey starts the existing arming (secrets_passkey_register) for the card's origin only. (named test; scene: The site wants a passkey instead)
- Approved, the passkey is stored under the card's name, granted to the teammate, with the card's Ask me setting. (named test; scene: The site wants a passkey instead)
- The credential card settles to "Stored as GH_PASSKEY for github.com" once made, and to Expired if the arming lapses unused. (named test; scene: The site wants a passkey instead)
- Neither card nor the agent ever sees the private key; the agent learns the name and kind (passkey). (describe_secret test; scene: The site wants a passkey instead)
- A credential card on a call thread shows on the desk and phone only; it is never answered by voice. (named test in hotline-voice; scene: On a call)
- The voice says once that it waits on the desk and phone, and nothing about values. (named test in hotline-voice; scene: On a call)
- A spoken yes or done does not answer a credential card. (named test in hotline-voice; scene: On a call)
- Once stored, the voice hears only "Stored as GITHUB_LOGIN" and says it in a few words. (named test; scene: On a call)
- make check passes.

## Log
- 2026-10-22T05:08:24.908Z: [codex] Read design, security method, scenario contract and epic decisions. Implementing a lease-bound credential card, one owner/phone capture command, metadata-only outcomes, existing vault/grant/delivery and passkey arming, and desk-only call routing. Same-turn repeat requests will be refused in core.
- 2026-10-22T05:23:10.693Z: [codex] Cross-repo acceptance gap found by reading hotline-computer/src/secrets.rs: Origin::permits accepts same scheme/port and same host OR subdomains. Desk capture binds sites to [the exact requested origin], but strict origin-only filling cannot be enforced by this desk branch; computer counterpart must tighten its origin check. Ask me is persisted/delivered here; use approval enforcement remains task-267/task-268 as planned.
