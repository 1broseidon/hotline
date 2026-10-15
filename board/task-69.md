---
id: task-69
title: "Desktop: Live calls picks one model from a list grouped by provider"
column: review
position: 6
priority: medium
tags:
  - voice
  - calls
  - settings
  - 0.38.0
relatedFiles:
  - ui/src/components/UseFor.tsx
  - ui/src/useFor.ts
parentId: epic-6
createdAt: "2026-10-15T18:50:11.063Z"
---

## Description
Done 2026-10-15 (a1e990f6 and later), seen by George. Live calls is its own section (spacing fixed). 'Calls talk with' + 'Signs in with' are one Model picker grouped by provider: xAI > Grok Voice (Included with your Grok subscription / $0.08 a minute on your xAI key), OpenAI > GPT-Live (Included with your ChatGPT subscription), GPT Realtime (About $0.10 a minute on your OpenAI key), Hotline > Legacy voice. A front with nothing connected is still listed with what to connect. Voice row beneath. Files: ui/src/components/UseFor.tsx, ui/src/useFor.ts.
