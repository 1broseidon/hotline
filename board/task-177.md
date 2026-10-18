---
id: task-177
title: "Onboarding: after Add teammate, open the new teammate's conversation"
column: todo
position: 0
priority: medium
tags:
  - onboarding
  - ui
  - qa
createdAt: "2026-10-18T21:49:00.734Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T21:49:27.349Z"
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
assignee: claude
---

## Description
Found in 0.40 QA (2026-10-18, Linux desktop, fresh room). The wizard's last step says 'Give it a name and say what it should help with. Then say hello.', but after 'Add teammate' the window shows the empty 'Pick a teammate on the left.' instead of opening the new teammate. Open its conversation, with the composer focused and the starter prompts showing. Test in ui/tests (the onboarding flow ends on the new teammate's conversation).
