---
id: task-75
title: "Desk: a call keeps its own transcript and posts summaries to the chat"
column: todo
position: 2
priority: high
tags:
  - voice
  - front
  - record
parentId: epic-7
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/duplex.rs
      description: Summary at hang-up and on an interval, no per-turn mirror
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: DM holds one summary per call
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:02:49.790Z"
createdAt: "2026-10-16T14:02:49.796Z"
dependsOn:
  - task-76
updatedAt: "2026-10-16T14:03:32.785Z"
---

## Description
Stop mirroring each front turn into the teammate's DM. The call's own record (voice/record.rs, the call card's transcript) keeps what was said. At hang-up, and every 5 minutes on a longer call, post one summary to the DM: what was asked, what was done (tools run, work handed off), anything left open, tagged as from the call. A post_to_chat front tool posts on request ('put that in the chat'). Work handed to the working session still runs in the open chapter as today. Keep the follow-up for a question the call ended on. Files: voice/duplex.rs, voice/spoken.rs, voice/record.rs, voice/mod.rs (Origin). Harness: a call with three exchanges and one hand-off leaves exactly the hand-off's turn plus one summary in the DM.
