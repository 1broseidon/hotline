---
id: task-81
title: "Composer: with no call key, the mic sits flush right"
priority: medium
tags:
  - bug
  - ui
  - composer
  - 0.39.0
contract:
  status: delivered
  deliverables:
    - type: file
      path: ui/src/components/Composer.tsx
      description: No hidden Send holding the slot
    - type: test
      path: ui/tests/composer.test.tsx
      description: Mic is the last key when there is no call key
  validation:
    commands:
      - make ui-check
  constraints:
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T15:07:41.722Z"
    pickedUpAt: "2026-10-16T15:09:52.253Z"
    reworkCount: 0
    deliveredAt: "2026-10-16T15:10:57.816Z"
    duration: 66
createdAt: "2026-10-16T15:07:41.728Z"
landsOn: release/0.39.0
updatedAt: "2026-10-16T15:11:25.470Z"
dispatch:
  agent: claude
  model: sonnet
  by: cli
  at: "2026-10-16T15:09:15.727Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T15:09:18.256Z"
    ended: "2026-10-16T15:11:25.470Z"
    outcome: done
assignee: claude
completedAt: "2026-10-16T15:11:21.273Z"
---

## Description
George 2026-10-16 (screenshot): when calls are unavailable, the composer's mic stops short of the right edge with an empty key-sized gap after it. Cause (ui/src/components/Composer.tsx, the right slot): with no call key and an empty field, the Send key still renders, hidden by opacity (.composer-send without data-shown is opacity 0 and pointer-events none) but still taking its 28px plus the gap. With a call key the slot is the call key, so nothing shows wrong. Fix: when there is no call key, the empty field's Send takes no space (render it only with a draft, or collapse its width while hidden and keep the arrival animation), so the mic sits at the right edge with the composer's usual inset, and typing brings Send in beside it. Check: empty field with calls unavailable, empty field with calls available (call key unchanged), typing (Send arrives, no jump), dictating, working with no draft (Stop).

## Log
- 2026-10-16T15:10:57.463Z: [claude] Send now renders only with a draft (arrival animation kept as a keyframe), so no hidden key holds the slot beside the mic; new ui/tests/composer.test.tsx covers mic-last and call-key unchanged. ui typecheck and tests pass. On branch dispatch/task-81.
- 2026-10-16T15:11:19.599Z: Landed on release/0.39.0
