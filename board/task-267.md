---
id: task-267
title: "Desk: each login and passkey is automatic or ask me, with an approval card for ask-me uses"
column: review
position: 2
priority: high
tags:
  - credentials
  - policy
parentId: epic-24
createdAt: "2026-10-22T03:09:08.664Z"
updatedAt: "2026-10-22T07:02:54.015Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T06:26:02.881Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 98520
    at: "2026-10-22T06:26:05.779Z"
assignee: codex
decisions:
  - decision-34
---

## Description
Each login and passkey is Automatic or Ask me. StoredSecret's sidecar (vault/shared.rs About) and the contract's SharedSecret gain use: Auto|Ask; keys stored before this are Automatic. Settings → Secrets (ui/src/components/Secrets.tsx) and the capture card set it. It rides to the computer in wire_form (computer/secrets.rs). For a held use the desk raises one approval card, SecretUseAsk {secret, kind, site, path, teammate}, on desk and phone (hotline-mobile task-50), with Approve, Allow 10 min (one key, one site, one teammate) and Deny, and carries the answer to the computer's door (task-268). No built-in list of sensitive sites. Not answered by voice.

Acceptance set: docs/design/credential-scenarios.html, scenes 5–6. Decisions: epic-24 note of 2026-10-21.

Done when:
- Approved, the passkey is stored under the card's name, granted to the teammate, with the card's Ask me setting. (named test; scene: The site wants a passkey instead)
- An ask-me key's use raises one approval card, on the desk and the phone, and its answer reaches the computer. (named test against the computer's fake door; scene: An ask-me key on github.com)
- Allow 10 min lets later uses of that key on that site through with no card until it lapses. (named test; scene: An ask-me key on github.com)
- Deny tells the agent plainly that the person declined. (named test; scene: An ask-me key on github.com)
- If the page stops waiting first, the card expires on both seats and the agent hears that it lapsed. (named test; scene: An ask-me key on github.com)
- An automatic key's use raises no card and is not held. (named test; scene: An automatic key on a low-stakes site)
- Settings → Secrets shows each key as Automatic or Ask me and changes it; the choice survives restart. (UI test plus vault test; scene: An automatic key on a low-stakes site)
- Keys stored before this change are Automatic. (vault migration test; scene: An automatic key on a low-stakes site)
- The setting rides to the computer with the set (wire_form); a change reaches it without a restart and applies from the next use. (named test; scene: An automatic key on a low-stakes site)
- make check passes.
The computer door (shared contract between task-267 and task-268; both build to exactly this):
- PUT /secrets: a login or passkey entry may carry "use": "auto" | "ask". Absent means auto. A variable never carries it.
- GET /secrets/uses (bearer): {"uses": [{"id": "<opaque>", "secret": "GH_PASSKEY", "kind": "passkey" | "login", "site": "github.com", "path": "/login", "askedAt": <ms>, "expiresAt": <ms>}]}, the uses the computer is holding now, oldest first. Empty list when none.
- POST /secrets/uses/answer (bearer): {"id": "...", "answer": "approve" | "allow" | "deny"}. "allow" approves this use and lets later uses of the same secret on the same site through without holding for 10 minutes. 200 with {"state": "approved" | "denied"}; 409 when the id is not held (answered, expired or unknown); 400 for a bad body.
- A held passkey get waits until answered or until the page's WebAuthn timeout (capped at 5 minutes), then is refused and the use expires.
- browser fill with secret on an ask-me login does not wait: it holds a use and answers at once with an error saying it is waiting for the person to approve NAME on SITE (use id), and to try again after they answer. The retry after "approve" fills once; after "allow" fills for 10 minutes.
- The desk polls GET /secrets/uses every 2 s for each running computer whose granted set has an ask-me key, raises one SecretUseAsk card per held use id, and settles the card as Expired when the id disappears unanswered.
- An older computer answers 404 on /secrets/uses: the desk then treats every key as automatic for it and says so once on stderr.

## Log
- 2026-10-22T06:35:35.531Z: [codex] Implemented enum use policy through vault sidecars and computer delivery, Settings controls, and secret_use_ask cards with desk/phone answer command. Adding fake-door acceptance and security regressions; capture Ask me input remains compatible and maps into the enum.
- 2026-10-22T07:02:53.752Z: [codex] Implemented Auto/Ask vault policy, live computer delivery, Settings controls, shared desk/phone use cards and answers, expiry/denial delivery, ten-minute allowance forwarding, and old-computer fallback. Added fake-door, vault, UI, restart and voice-denial proofs. make check and all 8 make precheck legs pass. Committed on dispatch/task-267 (HEAD 93e9bd27); clean worktree. Handoff: review this branch with the task-268 door and task-50 phone contract.
