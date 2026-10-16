---
id: task-82
title: "Providers: Live calls and Images can be turned Off"
column: review
position: 3
priority: medium
tags:
  - settings
  - voice
  - images
  - ui
  - 0.39.0
contract:
  status: delivered
  deliverables:
    - type: file
      path: ui/src/components/UseFor.tsx
      description: Off on both pickers
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: Calls off refuses a call and says why
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T15:22:42.062Z"
    pickedUpAt: "2026-10-16T15:39:25.462Z"
    reworkCount: 1
    deliveredAt: "2026-10-16T15:43:33.802Z"
    duration: 248
createdAt: "2026-10-16T15:22:42.069Z"
landsOn: release/0.39.0
updatedAt: "2026-10-16T15:46:06.758Z"
dispatch:
  agent: claude
  model: sonnet
  by: cli
  at: "2026-10-16T15:25:15.967Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T15:38:51.186Z"
assignee: claude
---

## Description
George 2026-10-16: some people will not want live calls or image generation at all. Settings › Providers › Use for gets an Off choice on both pickers, last in each list and never the default (a new or unset desk keeps today's behaviour). Live calls Off: the desk offers no calls, to the desk or a teammate: voice.status reports calls unavailable with a reason ('Calls are off in Settings'), so the window's composer call key and dock desk key are not drawn (no fake UI) and the phone draws no call key either (its capability check already follows voice.status). This is distinct from Legacy voice, which still calls. Images Off: no teammate is offered generate_image (Hotline Agent's tool list and the MCP tool both refuse with 'Image generation is off in Settings'), avatar generation for a new teammate is offered without a picture, and the Images picker shows Off. Stored in the room's settings beside the existing picks (voice.front provider 'off'? or a separate enabled flag: pick the shape that an older desk reads as the default, and record it in docs/voice.md and the images doc). Mirror on the phone: hotline-mobile Voice & images lists Off in both choice pages (separate mobile task once the desk ships). Harness: with Live calls Off, voice.call_start is refused and voice.status says why; with Images Off, generate_image is not in a teammate's tools.

## Log
- 2026-10-16T15:38:30.044Z: [claude] Added Off to Live calls (voice.front provider 'off') and Images (images provider 'off'), last in each picker and never the default. Off calls: voice.status says 'Calls are off in Settings' with no capabilities (call keys vanish), call_start refused; Legacy voice unchanged. Off images: generate_image dropped from Hotline Agent, MCP listing and tool ledger, refused if called, New teammate offers no picture. Harnesses: tests/voice.rs, tests/mcp.rs; docs in voice.md and wire.md; make check passes. Residual: an older desk reads 'off' calls as Legacy voice (still calls). On branch dispatch/task-82.
- 2026-10-16T15:38:42.326Z: Landing sent back (1 of 1): dispatch/task-82 conflicts with release/0.39.0 in crates/hotline-core/tests/voice.rs. On release/0.39.0 since this branch started: fb886e40 A call's summary is the teammate's own first-person words, and its next turn carries on from it (task-80). Merge release/0.39.0 into this branch (git merge release/0.39.0), keep what both sides meant, commit the merge, and run the checks: make check.
- 2026-10-16T15:43:33.474Z: [claude] Merged release/0.39.0 into dispatch/task-82; the one conflict (tests/voice.rs) kept both calls_off test and task-80's is_call_post helper; make check passes. On branch dispatch/task-82.
- 2026-10-16T15:46:06.758Z: Landed on release/0.39.0
