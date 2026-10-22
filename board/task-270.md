---
id: task-270
title: "OS-vault passkeys: keep a teammate's passkey in the person's own provider and sign each use on the host"
column: todo
position: 6
priority: low
tags:
  - credentials
  - macos
  - windows
parentId: epic-24
createdAt: "2026-10-22T03:09:10.304Z"
---

## Description
Only if the spike says yes. When the computer gets a WebAuthn request for a key marked os-vault, the extension relays it through the service to the desk, the desk asks the OS, the person confirms with Touch ID or Windows Hello, and the assertion goes back. Making a passkey the same way stores it in the person's provider.

Done when (to refine from the spike and deck):
- A github.com passkey kept in the person's provider signs in on the computer after a Touch ID confirmation: recorded manual check.
- No private key reaches the computer or the desk's vault: named test on the relay path.
