---
id: task-268
title: "Computer (hotline-computer): hold an ask-me key's use until approved, and show it on the page"
column: todo
position: 4
priority: high
tags:
  - credentials
  - policy
  - hotline-computer
parentId: epic-24
createdAt: "2026-10-22T03:09:09.205Z"
---

## Description
In ~/Projects/personal/hotline-computer (ext/v3 and later). The bridge (src/bridge.rs answer: get, fill) holds a request whose secret is use: ask, and exposes it through a bearer door, GET /secrets/uses and POST /secrets/uses/answer, alongside /passkeys/registration. browser fill on an ask-me login answers at once that it is waiting for the person (with an id), so no tool call runs past 50 s; the agent retries after the answer. A held passkey request waits within the page's WebAuthn timeout. The extension card on the page shows 'Waiting for approval at the desk' and then Approved or Declined; its record notes each.

Also for capture: when a new key for the open site arrives (PUT /secrets), the card on that page shows it briefly ('New login for this site: NAME').

Done when:
- A held passkey get completes after approval and is refused after a denial: contract test (tests/contract.rs) with the passkey proof page.
- browser fill on an ask-me login returns waiting, then fills after approval: contract test.
- The extension card shows waiting and the outcome: screenshots in the QA notes.
- make check passes; the guide (skills/hotline-computer/SKILL.md) explains ask-me keys to agents.
