---
id: task-256
title: "Legacy voice: a tool-first turn says nothing for a minute instead of its own first sentence"
column: in-progress
position: 1
priority: medium
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T08:37:51.638Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T08:38:48.810Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T08:38:50.424Z"
assignee: codex
updatedAt: "2026-10-21T08:39:11.805Z"
---

## Description
Linux QA, 2026-10-21, Legacy voice, Rex on Grok 4.7, a180a43d: 'Search the web for how tall the Bell Rock lighthouse is' went straight to ~20 web_search calls with no spoken first sentence for over a minute (the transcription misheard the name, which made it long). The voice contract asks for a short spoken sentence before a tool; this model ignored it. Whether the one soft cue played can't be heard on the QA computer. Done when: on Legacy, a turn whose first output is a tool call gets the agent's own short sentence or the single soft cue within a second or two, and a long tool run gets an occasional short progress line rather than a minute of silence; checked with Hotline Agent on Grok and an ACP agent.
