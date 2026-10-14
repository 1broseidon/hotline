---
id: task-59
title: "Desk: front-to-teammate bridge"
column: todo
position: 39
priority: high
tags:
  - voice
  - calls
  - 0.37.0
parentId: epic-6
dependsOn:
  - task-58
createdAt: "2026-10-14T20:03:56.919Z"
---

## Description
Front tools: send_to_teammate(words) passes the person's own words, unrephrased, into the teammate's session as start-or-steer in its open chapter (same path, grants and chapter as a typed message; voice origin kept); remain_silent. Teammate side: start/end fragments like Codex's realtime_start/end ('behind an intermediary; input is a transcript and may have recognition errors'; 'call ended, resume normal behaviour'). Back to the front: short progress notes while the turn runs, the reply's <spoken> line spoken verbatim (inject as text the front must say, or client-side speech), approvals/questions become 'it needs you to approve on the desk/phone'. A second send while a turn runs steers it. Front never reports work the teammate did not.
