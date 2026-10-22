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
updatedAt: "2026-10-22T05:06:08.681Z"
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
The computer door (shared contract between task-267 and task-268; both build to exactly this):
- PUT /secrets: a login or passkey entry may carry "use": "auto" | "ask". Absent means auto. A variable never carries it.
- GET /secrets/uses (bearer): {"uses": [{"id": "<opaque>", "secret": "GH_PASSKEY", "kind": "passkey" | "login", "site": "github.com", "path": "/login", "askedAt": <ms>, "expiresAt": <ms>}]}, the uses the computer is holding now, oldest first. Empty list when none.
- POST /secrets/uses/answer (bearer): {"id": "...", "answer": "approve" | "allow" | "deny"}. "allow" approves this use and lets later uses of the same secret on the same site through without holding for 10 minutes. 200 with {"state": "approved" | "denied"}; 409 when the id is not held (answered, expired or unknown); 400 for a bad body.
- A held passkey get waits until answered or until the page's WebAuthn timeout (capped at 5 minutes), then is refused and the use expires.
- browser fill with secret on an ask-me login does not wait: it holds a use and answers at once with an error saying it is waiting for the person to approve NAME on SITE (use id), and to try again after they answer. The retry after "approve" fills once; after "allow" fills for 10 minutes.
- The desk polls GET /secrets/uses every 2 s for each running computer whose granted set has an ask-me key, raises one SecretUseAsk card per held use id, and settles the card as Expired when the id disappears unanswered.
- An older computer answers 404 on /secrets/uses: the desk then treats every key as automatic for it and says so once on stderr.
