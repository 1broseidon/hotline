---
id: task-204
title: "Voice: acknowledgements that fit, never repeat, and never leave dead air; no guessing"
column: backlog
position: 40
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/assets
parentId: epic-21
createdAt: "2026-10-19T13:59:38.640Z"
dependsOn:
  - task-201
updatedAt: "2026-10-19T20:00:16.276Z"
---

## Description
Replace the example phrases (front_prompt.rs) with a description of a good acknowledgement (a few words naming what was asked) and a short list of what not to say (generic filler, "let me check that for you", "passing that on", a repeat). Feed the call's last few acknowledgements back into the front as already said. When a hand-off goes out and the front said nothing, the desk plays a short soft sound on the call (both fronts, desktop and phone), not words. Tighten the rules so conversational follow-ups and clarifications stay on the call and the front never answers a question about code, state or results it hasn't looked up. Tests: the prompt carries no stock acknowledgement; recent acknowledgements reach the front; a silent hand-off plays the sound once.
