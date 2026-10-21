---
id: task-239
title: "Cards on a call: what needs the person is said on the call, and approvals and simple questions are answered by voice"
column: in-progress
position: 0
priority: high
tags:
  - voice
  - security
parentId: epic-21
createdAt: "2026-10-20T20:13:08.678Z"
dependsOn:
  - task-225
updatedAt: "2026-10-21T07:29:29.477Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T06:53:08.339Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T07:29:09.117Z"
assignee: codex
decisions:
  - decision-15
---

## Description
Scenario: Something needs you during the call. Decision (George accepted the deck's lean, 2026-10-20): approvals and simple questions may be answered by voice, and nothing that needs a credential, a 2FA tap, a screen, a payment or a deletion may.

Done when:
- A card that would wake the person in the chat is said on the call instead, once, at a natural break, not over the person.
- An approval or a simple question is answered by a yes or no on the call, and the answer is written to the chat as a record, from the call.
- A card needing a credential, a 2FA tap, a screen, a payment or a deletion is said and never answered by voice. It waits for the desk.
- The card handler enforces which cards may be answered by voice, never the model, as a capability change under docs/security.md. It's proved allowed and refused through the real handler.

## Log
- 2026-10-21T07:00:49.571Z: [codex] Implementing a stable-ID call card queue and core-only answer path. Existing action/passkey/opaque driver cards fail closed at the desk; core-owned collaboration approvals allow only a live session. Adding a separate informational yes/no question path with no operation authority, lease checks, and from-call records; validating through real room/wire handlers.
- 2026-10-21T07:26:47.737Z: [codex-task-239] Implemented queued once-only call announcements, caller-floor gating, core-owned yes/no settlement with from-call chat records, informational ask_question cards, and active-call push routing. Core handler refuses action/passkey/opaque driver cards plus forged and stale authority. make check passed, including both drivers, three live provider seams and own call-thread continuation; running precheck before committing for Review. Cymbal reports the scope across call runtime, card/session handler, contract/wire and desktop UI.
- 2026-10-21T07:28:56.525Z: [codex-task-239] Ready for review on dispatch/task-239, commit fff9ffef (clean worktree). Implemented once-only queued call announcements at conversational gaps, caller-floor signaling, core-enforced session collaboration approvals and informational yes/no questions, from-call chat records, resolved-card UI and active-call push suppression. Wired main and own call-thread cards into both drivers and live fronts. make check, all 8 make precheck legs and commit hooks passed. Boundary/default/grant/enforcement/tests/residuals are in docs/security.md and decision-15. Left unwired outside this tree: older/mobile whole-clip clients must adopt voice.input_state and card.resolved; physical microphone/playback timing remains task-206's device pass. Opaque driver approvals intentionally remain desk-only pending trusted operation metadata; informational answers never grant operation permission.
- 2026-10-21T07:29:08.591Z: Landing sent back (1 of 1): dispatch/task-239 conflicts with release/0.42.0 in docs/security.md. On release/0.42.0 since this branch started: 8f905bce Call pointers require something worth returning to in chat (task-207). Merge release/0.42.0 into this branch (git merge release/0.42.0), keep what both sides meant, commit the merge, and run the checks.
