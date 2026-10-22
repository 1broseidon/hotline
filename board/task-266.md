---
id: task-266
title: "Desk: request_credential tool and an in-chat credential card whose values go only to the vault"
column: todo
position: 2
priority: high
tags:
  - credentials
parentId: epic-24
createdAt: "2026-10-22T03:09:08.125Z"
---

## Description
A teammate tool request_credential {site, reason, name?, kind: login|passkey} raises a credential card in the chat (desk). The card shows the site's origin, the reason and secure fields: username, password, and an optional one-time-code seed. Submit sends one command, secrets.capture.answer {action_id, values}, from the owner or phone seat (wire/seat.rs), and the desk:
- stores a Login {sites: [origin], username, password, totp?} under the chosen name (vault/shared.rs set_shared),
- ticks the grant for the asking teammate (PersonaComputer.secrets),
- re-sends the set (Room::secrets_changed → hand_secrets → computer/secrets.rs deliver),
- answers the agent: 'Stored as NAME for <origin>; sign in with browser fill secret NAME.username and NAME.password.'
kind passkey offers Make a passkey: the existing arming (secrets_passkey_register) with its approval card.

Contract: TranscriptEvent::CredentialAsk {id, site, reason, kind, status, stored_as?} with status Pending|Stored|Declined|Expired|Withdrawn; Waiting::Credential for pushes; a ThreadAnswer variant if answered from a thread. The tape holds the ask and its outcome, never a value.

Done when (refined from the deck):
- Submitting the card stores the login, grants it to the teammate and delivers it to the computer: named test in session (capture stores, grants and delivers).
- No value appears in the tape, the transcript JSON, the agent's answer or desk logs: named test that captures a known value and greps every sink.
- Decline and expiry answer the agent and store nothing: named tests.
- A credential card is never answered by voice and the voice front says it waits on the desk: named test in hotline-voice.
- The card renders on the desk with password fields that are cleared on submit or dismiss: UI test in Asks.
