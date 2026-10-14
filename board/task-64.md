---
id: task-64
title: "Desk: GPT-Live front over the ChatGPT login (stretch)"
column: todo
position: 44
priority: low
tags:
  - voice
  - calls
  - providers
  - openai
  - stretch
  - 0.37.0
parentId: epic-6
dependsOn:
  - task-61
createdAt: "2026-10-14T20:03:59.244Z"
---

## Description
Second provider. Desk POSTs the phone's SDP offer to chatgpt.com/backend-api/codex/realtime/calls (JSON {sdp, session}, model gpt-live-1-codex, delegation client, openai-alpha quicksilver=v2, bearer + chatgpt-account-id) and returns the answer; phone holds WebRTC media; desk attaches the sideband (wss://api.openai.com/v1/live/{call_id}, same identity) for delegation and context appends. Private route: feature-flag it, fail soft to Grok or the pipeline, and watch for an attestation requirement. API-key path (GA Realtime/GPT-Live) as the stable alternative.
