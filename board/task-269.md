---
id: task-269
title: "Spike: can the desk ask macOS (and Windows) for a passkey for any site?"
column: todo
position: 5
priority: medium
tags:
  - credentials
  - spike
  - macos
parentId: epic-24
createdAt: "2026-10-22T03:09:09.744Z"
updatedAt: "2026-10-22T04:35:55.763Z"
---

## Description
Can the desk ask macOS (and Windows) for a passkey for any site, so a passkey kept in the person's own provider (1Password, Bitwarden, iCloud Keychain) signs each use on the host, with the OS prompt as the approval and no per-vault code?

Questions:
- macOS: ASAuthorizationPlatformPublicKeyCredentialProvider for an rpId the app is not associated with needs com.apple.developer.web-browser.public-key-credential. Can Hotline get it, on what terms, and does it route to third-party credential providers?
- Windows 11: can a desktop app call the WebAuthn API for any rpId, and do third-party passkey managers answer it?
- Linux: state of a passkey provider API; expected answer, not yet.
- Can the teammate's own passkey be made there and used later the same way?

macOS comes before Windows (epic-24 decision 12).

Done when:
- docs/design/os-passkeys.md answers each question with evidence (docs, a signed test build, or Apple's reply) and ends in a yes or no for task-270.
