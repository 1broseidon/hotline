---
id: task-177
title: "Onboarding: after Add teammate, open the new teammate's conversation"
priority: medium
tags:
  - onboarding
  - ui
  - qa
createdAt: "2026-10-18T21:49:00.734Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T21:53:10.246Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T21:49:22.877Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 95396
    at: "2026-10-18T21:49:27.349Z"
    ended: "2026-10-18T21:53:10.246Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T21:53:09.786Z"
---

## Description
Found in 0.40 QA (2026-10-18, Linux desktop, fresh room). The wizard's last step says 'Give it a name and say what it should help with. Then say hello.', but after 'Add teammate' the window shows the empty 'Pick a teammate on the left.' instead of opening the new teammate. Open its conversation, with the composer focused and the starter prompts showing. Test in ui/tests (the onboarding flow ends on the new teammate's conversation).

## Log
- 2026-10-18T21:53:01.309Z: [claude] Cause: NewTeammateForm opened the teammate only after session.start answered, which waits for the whole start (seconds, or never if it fails), while the roster event had already taken the welcome away, so the window sat on 'Pick a teammate on the left.'. Now it opens on persona.create's answer and starts the session behind it (a failed start shows in the conversation as a notice and error state); the composer takes focus while the conversation is fresh (starters showing). Test: ui/tests/onboarding.test.tsx fails on the old code with QA's screen. Commit 873f59fc on dispatch/task-177.
- 2026-10-18T21:53:08.957Z: Landed on release/0.40.0
