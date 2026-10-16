---
id: task-80
title: "Desk: a call's summary is the teammate's own words, and its session knows what was said"
column: todo
position: 4
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
      path: crates/hotline-core/src/voice/record.rs
      description: Summary in the teammate's words
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: Summary carries the answer and enters the teammate's history
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T15:01:23.363Z"
createdAt: "2026-10-16T15:01:23.373Z"
dependsOn:
  - task-79
updatedAt: "2026-10-16T15:06:04.770Z"
---

## Description
Follow-up to task-75 (George's review 2026-10-16). task-75's summary is a mechanical log posted as an info notice: 'From the call: Asked: "…" Done: ran search_thread ×2; handed to the working session: "…"'. Three problems: (1) it omits what the front answered, so it is not a summary of the conversation; (2) it names internals (tool names, 'working session') and reads as the system, not the teammate, breaking the one-teammate goal of ADR 0005; (3) info notices never enter the teammate's history (session/chapters.rs only carries error notices), so after a call the working session does not know what was said, and a typed follow-up starts cold. Do: write the summary in the teammate's first person from the call's transcript (what was asked, what the front answered, what it looked up, what is still being worked on, what is left open), in plain prose with no tool names; post it as an agent message from the teammate tagged as from the call; and put it in the teammate's history so its next turn carries on from the call (the person's words and the front's answers, scrubbed and budgeted as front_prompt does). Decide and record (ADR 0005 open question) whether the front writes it (cheap, live) or the teammate's session is handed the transcript to write it; default to the front's words. post_to_chat posts as the teammate too. Harness: after a call that answered a question from search_thread, the DM shows one first-person summary with the answer and no tool name, and a typed follow-up turn's history includes the call.
