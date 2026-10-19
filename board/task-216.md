---
id: task-216
title: "Slice 1: Room::post, one door to the chat, posts stamped with where they came from"
column: in-progress
position: 12
priority: high
tags:
  - threads
  - chat
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - crates/hotline-contract
  - docs/adr/0008-one-agent-thread-layer.md
parentId: epic-20
createdAt: "2026-10-19T19:59:30.596Z"
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: "headless harness: a quiet run that posts, one that does not, and a call's post, read off the wire"
    - type: docs
      path: docs/design.md
      description: the agent event's optional from and scheduled, and the reference importer's reading of them
  validation:
    commands:
      - make check
      - "! git grep -n -e pending_calls -e 'escalation::follow_up' -- crates/hotline-core/src"
      - "! git grep -nF 'From the call: {text}' -- crates/hotline-core/src/voice/front_tools.rs"
      - git grep -qn 'fn post(' -- crates/hotline-core/src/session
  constraints:
    - "Tape byte-compatibility (AGENTS.md rule 4): the new fields are optional and additive; say so in design.md"
    - "Do not touch the post_to_chat tool or the Escalate trait: that is task-217"
    - "Do not touch session/peers or runner.rs: task-218 runs alongside"
  metrics:
    readyAt: "2026-10-19T20:03:13.285Z"
    pickedUpAt: "2026-10-19T20:03:43.502Z"
    reworkCount: 0
updatedAt: "2026-10-19T20:03:43.502Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:03:29.011Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:03:31.889Z"
---

## Description
ADR 0008 §3 and §6 slice 1, as written there: Room::post(teammate, from, post); call_posted, the quiet run's follow-up and send_file from a thread write through it; TranscriptEvent::Agent gains optional `from` and `scheduled` (the reference importer reads them; design.md says so); a quiet run's note is posted as written; posts reach the home agent by who asked (§3). Removes what slice 1 lists. Proved by the headless harness the ADR names.
