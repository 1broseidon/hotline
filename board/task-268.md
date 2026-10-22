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
updatedAt: "2026-10-22T04:35:55.259Z"
---

## Description
In ~/Projects/personal/hotline-computer (from ext/v3, rc.6). The bridge (src/bridge.rs answer: get, fill) holds a use of a key whose use is ask, and exposes it through a bearer door, GET /secrets/uses and POST /secrets/uses/answer, beside /passkeys/registration. browser fill on an ask-me login answers at once that it is waiting (with an id); the agent retries after the answer. A held passkey get waits within the page's WebAuthn timeout, then expires. The extension card on the page shows Waiting for approval at the desk, then Approved or Declined, and its record notes each. When a new key for the open site arrives in PUT /secrets, the card on that page shows New login for this site: NAME. Allow 10 min is honoured by the computer for that key and site. The guide (skills/hotline-computer/SKILL.md) explains ask-me keys to agents.

Acceptance set: docs/design/credential-scenarios.html (hotline repo), scenes 1, 5, 6. Decisions: epic-24 note of 2026-10-21.

Done when:
- When the set arrives, the card on the open page shows "New login for this site: GITHUB_LOGIN". (screenshot in the QA notes; scene: A login, asked for in the chat)
- A held passkey get completes after approval and is refused after a denial. (contract test with the passkey proof page; scene: An ask-me key on github.com)
- browser fill on an ask-me login answers at once that it is waiting, with an id, so no tool call runs past 50 s; the retry after approval fills. (contract test; scene: An ask-me key on github.com)
- The card on the page shows "Waiting for approval at the desk", then Approved or Declined; its record notes each. (screenshots in the QA notes; scene: An ask-me key on github.com)
- Each use still leaves a note on the page and a line in the card's record. (screenshot in the QA notes; scene: An automatic key on a low-stakes site)
- The setting rides to the computer with the set (wire_form); a change reaches it without a restart and applies from the next use. (named test; scene: An automatic key on a low-stakes site)
- make check passes in hotline-computer; a new rc image is built and driven over MCP, with screenshots in qa/.
