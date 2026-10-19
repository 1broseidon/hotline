---
id: task-216
title: "Slice 1: Room::post, one door to the chat, posts stamped with where they came from"
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
  status: delivered
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
    pickedUpAt: "2026-10-19T20:23:09.185Z"
    reworkCount: 1
    deliveredAt: "2026-10-19T20:30:24.113Z"
    duration: 435
updatedAt: "2026-10-19T20:32:03.963Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:22:54.860Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:22:58.046Z"
    ended: "2026-10-19T20:30:31.303Z"
    outcome: review
completedAt: "2026-10-19T20:32:03.963Z"
---

## Description
ADR 0008 §3 and §6 slice 1, as written there: Room::post(teammate, from, post); call_posted, the quiet run's follow-up and send_file from a thread write through it; TranscriptEvent::Agent gains optional `from` and `scheduled` (the reference importer reads them; design.md says so); a quiet run's note is posted as written; posts reach the home agent by who asked (§3). Removes what slice 1 lists. Proved by the headless harness the ADR names.

## Log
- 2026-10-19T20:22:47.691Z: [claude] Lead: try 1 ended before committing (it said make check was still running) — 35 files of uncommitted work sit in the task-216 worktree, branch has no commits. Next try: do not redo it; run make check in the foreground and wait for it to finish, fix what fails, then commit. task-218 has since landed on release/0.41.0 and rewrote session/peers (Room::thread_agent, Threads::drive); this diff also touches session/peers.rs and peers/tests.rs, so merge release/0.41.0 in, resolve keeping task-218's builder, rerun cargo test -p hotline-core unfiltered (AGENTS.md rule 3) and make check, then commit and deliver. Do not end the turn with a command still running.
- 2026-10-19T20:30:26.932Z: [claude] Committed try 1's work as b790b237 (Room::post in session/post.rs; call_posted, the quiet run's follow-up and send_file from a thread write through it; TranscriptEvent::Agent has optional from/scheduled, documented in design.md). Merged release/0.41.0 cleanly (3fb75707). session/peers only gained from: None/scheduled: None field passthroughs for the new optional fields. make check, make precheck, and the contract's validation greps all pass; harness: tests/schedule.rs a_quiet_run_is_heard_only_when_it_posts plus tests/voice/front_tools.rs. For task-217: post_to_chat and the Escalate trait are untouched, so they still go their old way and should move onto Room::post. Work is on branch dispatch/task-216.
- 2026-10-19T20:32:03.583Z: Landed on release/0.41.0
