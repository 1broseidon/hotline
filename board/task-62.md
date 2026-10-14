---
id: task-62
title: "Desk: record front calls truthfully and count their cost"
column: todo
position: 42
priority: medium
tags:
  - voice
  - calls
  - ledger
  - 0.37.0
parentId: epic-6
dependsOn:
  - task-59
createdAt: "2026-10-14T20:03:58.314Z"
---

## Description
Front transcript (person and front lines) goes to calls/<callId>.jsonl and the DM call marker, kept apart from the teammate's tape, which holds only what was handed to it and what it answered. Ledger: per-minute cost for API-key sessions (Grok $0.08/min, GPT-Live $0.05/min), zero for subscription logins, shown in voice status; budget checks before a call starts. Note subscription limits once known.
