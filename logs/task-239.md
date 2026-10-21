---
id: task-239
title: "Cards on a call: what needs the person is said on the call, and approvals and simple questions are answered by voice"
priority: high
tags:
  - voice
  - security
parentId: epic-21
createdAt: "2026-10-20T20:13:08.678Z"
dependsOn:
  - task-225
updatedAt: "2026-10-21T16:42:22.870Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T14:39:34.824Z"
  until: done
  tries: 3
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T14:39:36.383Z"
    ended: "2026-10-21T14:53:43.441Z"
    outcome: review
assignee: codex
decisions:
  - decision-15
  - decision-27
completedAt: "2026-10-21T16:42:22.870Z"
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
- 2026-10-21T07:34:59.417Z: [codex-task-239] Resolved the requested release/0.42.0 merge on dispatch/task-239 in 19662d10; worktree clean. Kept task-207's automatic written-result security record and task-239's core-only voice-card boundary intact. make check and all 8 make precheck legs passed, as did commit hooks. Ready for reviewer to merge. Nothing newly left unwired: older/mobile clients still need voice.input_state and card.resolved adoption; task-206 owns physical microphone/playback verification; opaque driver approvals remain desk-only pending trusted metadata.
- 2026-10-21T07:35:10.810Z: Landed on release/0.42.0
- 2026-10-21T14:39:33.468Z: [claude] Reopened into To do: George desk QA 2026-10-21 09:31: a card raised on a call lands in the call's thread, not the home chat where the operator sees it. Rework.
- 2026-10-21T14:39:33.967Z: [claude] 2026-10-21 Pork Chop, rework brief from George's desk QA (call 4be175cf, 09:29, from his phone): he asked on a call for a yes/no card; the clone's ask_question wrote the human_action card into the CALL thread. George, 09:31: it should be sent to the home thread where the operator would actually see it. The voice also told him 'I put a yes-or-no card in the chat', which was false.

Done when (in addition to the original lines):
- A card raised from a call's thread (by the clone or the teammate, any kind: question, action, permission, passkey) appears in the teammate's home chat, stamped 'From the call' with the call's title and a link to the call, and notifies like any card when the call is down.
- Answering it from the home chat settles it; answering by voice on the call settles the same card; either way it shows settled in both places, and the 'Answered on the call: …' record sits in the home chat beside it.
- The call's thread still shows that a card was raised (a line linking the home-chat card), so the transcript stays whole.
- The card handler's voice permissions (decision-15, docs/security.md) are unchanged: what may be answered by voice is still the core's decision.
- Tests: a call-thread ask_question lands in the home tape with from=call; home-chat thread.answer settles it and the voice queue drops it; a voice answer settles the home-chat card.
Merge release/0.42.0 first (now 3d590335), make check in the foreground, commit before done.
- 2026-10-21T14:48:43.003Z: [codex-task-239] Rework routes call-thread cards to the home tape with core-stamped call provenance and the same ID in the call transcript. Home answers reach the original driver/wait and resolve the voice queue; collaboration approvals also carry their call origin. Both-driver question tests and driver permission routing pass; UI call/home links pass. Running make check in the foreground, then precheck and commit on dispatch/task-239.
- 2026-10-21T14:53:43.956Z: Not landed: 23 files are not committed in the worktree: send it back, or commit in your editor
- 2026-10-21T16:42:22.489Z: Landed on release/0.42.0
