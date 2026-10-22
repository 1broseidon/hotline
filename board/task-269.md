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
---

## Description
Ask the OS to sign for an arbitrary rpId, so a passkey kept in the person's own provider (1Password, Bitwarden, iCloud Keychain) can sign each use on the host, with the OS prompt as the approval, and with no per-vault code.

Questions:
- macOS: ASAuthorizationPlatformPublicKeyCredentialProvider (and the security-key provider) for an rpId the app is not associated with needs com.apple.developer.web-browser.public-key-credential. Can Hotline get it, under what conditions, and does it route to third-party credential providers?
- Windows 11: can a desktop app call the WebAuthn API (webauthn.dll) for any rpId, and do third-party passkey managers answer it?
- Linux: state of a passkey provider API (credentials portal proposals); expected answer: not yet.
- Creating: can the agent's own passkey be made there (stored in the person's provider) and used later the same way?

Done when:
- docs/design/os-passkeys.md answers each question with evidence (docs, a signed test build, or Apple's reply), and ends in a yes or no for building it.
