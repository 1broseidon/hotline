---
id: task-217
title: "Slice 2: post_to_chat on the desk's MCP server for both drivers; tell_person and Escalate go"
column: in-progress
position: 13
priority: high
tags:
  - threads
  - chat
  - security
assignee: claude
relatedFiles:
  - crates/hotline-core/src/mcp/server.rs
  - crates/hotline-core/src/driver.rs
  - crates/hotline-core/src/driver/rig
  - docs/security.md
parentId: epic-20
createdAt: "2026-10-19T19:59:31.099Z"
dependsOn:
  - task-216
  - task-218
updatedAt: "2026-10-19T20:50:46.481Z"
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: every refusal row of ADR 0008 §3 (loud DM turn, a subagent run, a second post, a second wake, a revoked lease, another teammate's chat, over 2,000 chars) through the real handler on both drivers, and an ACP quiet run that posts
    - type: docs
      path: docs/security.md
      description: post_to_chat's default, grant source, enforcement points, tests and residual risk
  validation:
    commands:
      - make check
      - "! git grep -n -e escalate_next -e 'trait Escalate' -e tell_person -- crates/hotline-core/src"
      - git grep -qn post_to_chat -- crates/hotline-core/src/mcp
      - git grep -qn post_to_chat -- docs/security.md
      - cargo test -p hotline-core --test mcp
  constraints:
    - "A capability change: follow docs/security.md"
    - Write through Room::post from task-216; no second door to the chat
  metrics:
    readyAt: "2026-10-19T20:03:13.865Z"
    pickedUpAt: "2026-10-19T20:32:52.379Z"
    reworkCount: 0
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:32:34.969Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:32:37.885Z"
    ended: "2026-10-19T20:50:46.481Z"
    outcome: review
---

## Description
ADR 0008 §3 and §6 slice 2: post_to_chat(text, wake?) on Hotline's MCP server, same handler in-process for Hotline Agent; the gate (kind, modality, once a turn, one wake a turn, lease, 2,000 chars); a subagent the home agent launched is refused. security.md: default, grant source, enforcement, tests (every refusal row in the ADR's table on both drivers), residual risk. Removes the Escalate trait, escalate_next, the Rig-only tell_person and per-turn arming. An ACP quiet run that posts is proved.

From task-216 (landed on release/0.41.0): Room::post lives in crates/hotline-core/src/session/post.rs, and call_posted, the quiet run's follow-up and send_file from a thread already write through it. post_to_chat and the Escalate path were left alone and still reach the chat their old way: move them onto Room::post (stamp `from` with the thread, `scheduled` where a job asked), so no second door to the chat is left. task-218's pairs build through Room::thread_agent, so wire a pair's thread kind for the gate there, not in peer_session.

## Log
- 2026-10-19T20:51:28.807Z: [claude] Lead: try 1 ended its turn while `make check` was still running, so nothing was validated. Its work is on dispatch/task-217 (4c256848, 4a68b1d6) with uncommitted edits to session/post.rs and session/prompt.rs. Next try: keep that work, run `make check` in the foreground (generous timeout, never backgrounded) and wait for it, fix what fails, commit, then run every contract validation command before delivering. Do not end the turn while a command is pending.
