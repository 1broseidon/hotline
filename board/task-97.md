---
id: task-97
title: "Desk health and reachability: Hotline Agent unavailable, a reachability row in Settings › Remote, and a desk.status view"
column: backlog
position: 43
priority: medium
tags:
  - remote
  - health
  - settings
createdAt: "2026-10-17T06:22:23.177Z"
---

## Description
Split out of task-12 on 2026-10-17 (epic-10 scoped task-12 to the device marker, the desk's kind and version, and the public endpoint). Three items carried over from older tasks:
1. Report Hotline Agent unavailable when no provider is connected.
2. Show desk reachability away from home in Settings › Remote, with the Relay row shown before a server is paired (from task-29).
3. Add a desk.status view (version, uptime, runtime, disk, failing logins/schedules, last error) for the phone banner and `hotline status`, each problem with its fix (from task-30).

## Log
- 2026-10-19T01:32:26.273Z: [claude] Follow-ups from task-188 (epic-14), confirmed in QA on the Linux desk 2026-10-18: (1) Connecting a provider doesn't restart sessions waiting for one; only the next message clears the 'error' band. (2) The composer's no-provider refusal is text only, with no Settings link. (3) QA: 'Hotline Agent unavailable' shows only after you send, not when the conversation opens. (4) QA: with the vault's login gone, Settings › Providers still reads 'xAI · Signed in · All models' while the agent says no provider is connected.
- 2026-10-19T01:32:32.538Z: [claude] Correction to the note above: only items (3) and (4) were seen in QA; (1) and (2) are the lead's notes from task-188 and weren't exercised.
