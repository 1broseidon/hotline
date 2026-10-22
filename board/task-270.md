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
updatedAt: "2026-10-22T04:35:56.270Z"
---

## Description
Only if task-269 says yes. When the computer gets a WebAuthn request for a key kept in the person's provider, the extension relays it through the service to the desk, the desk asks the OS, the person confirms with Touch ID or Windows Hello, and the assertion goes back. The passkey card offers Keep in your passwords app as a second choice to Make a passkey. No phone fallback when the Mac is away. Passwords and Linux hosts keep using capture.

Acceptance set: docs/design/credential-scenarios.html, scene 8. Decisions: epic-24 note of 2026-10-21.

Done when:
- A github.com passkey kept in your provider signs in on the computer after a Touch ID confirmation. (recorded manual check; scene: A passkey in your own passwords app)
- No private key reaches the computer or the desk's vault. (named test on the relay path; scene: A passkey in your own passwords app)
- The OS prompt is the approval: no Hotline card is raised in addition. (named test; scene: A passkey in your own passwords app)
- If nobody confirms before the page stops waiting, the site is refused and the agent is told it lapsed. (named test; scene: A passkey in your own passwords app)
