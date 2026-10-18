---
id: task-178
title: Call keys appear on a fresh desk as soon as calls are ready, not after the first exchange
column: review
position: 0
priority: medium
tags:
  - voice
  - ui
  - qa
createdAt: "2026-10-18T21:49:01.284Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T21:53:14.543Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T21:49:23.983Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 95396
    at: "2026-10-18T21:49:29.213Z"
assignee: claude
---

## Description
Found in 0.40 QA (2026-10-18, Linux, fresh room, xAI SuperGrok signed in during onboarding, Live calls = Grok Voice). Right after onboarding, Rex's composer had no call key and the rail's dock had no 'Call the desk'. Both appeared only after the first couple of exchanges. Find what the window waits on (voice.status / readiness after a provider is added mid-session, or a subscription that isn't refreshed after onboarding) and make the keys appear as soon as a call could be placed. Only when it really can: no fake UI. Test over the wire: adding a voice-capable provider updates readiness without a new turn.

## Log
- 2026-10-18T21:53:14.188Z: [claude] Cause: the core computes voice.status fresh each time, but the window only asked it again on focus, on a voice-setting change, or when Settings › Providers called recheckVoiceSupport. Onboarding's sign-in did none of these. Fix: useVoiceSupport now also asks again on the room's modelsRevision, which goes up on every credential or models event, so a sign-in from anywhere counts. I deleted recheckVoiceSupport because it is now redundant. New wire test connecting_a_voice_provider_readies_calls_without_a_turn (tests/voice/direct.rs): adding an xAI key sends a credential event on the room, and voice.status is ready, with the Grok front, with no turn in between. make ui-check passes. Committed on dispatch/task-178 (1d07b525).
