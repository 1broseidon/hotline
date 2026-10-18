---
id: task-178
title: Call keys appear on a fresh desk as soon as calls are ready, not after the first exchange
column: todo
position: 1
priority: medium
tags:
  - voice
  - ui
  - qa
createdAt: "2026-10-18T21:49:01.284Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T21:49:24.115Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T21:49:23.983Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Found in 0.40 QA (2026-10-18, Linux, fresh room, xAI SuperGrok signed in during onboarding, Live calls = Grok Voice). Right after onboarding, Rex's composer had no call key and the rail's dock had no 'Call the desk'. Both appeared only after the first couple of exchanges. Find what the window waits on (voice.status / readiness after a provider is added mid-session, or a subscription that isn't refreshed after onboarding) and make the keys appear as soon as a call could be placed. Only when it really can: no fake UI. Test over the wire: adding a voice-capable provider updates readiness without a new turn.
