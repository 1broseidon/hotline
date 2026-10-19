---
id: task-219
title: "Slice 3: files and images from any thread land in the chat"
column: in-progress
position: 15
priority: medium
tags:
  - threads
  - chat
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session/files.rs
parentId: epic-20
createdAt: "2026-10-19T19:59:32.080Z"
dependsOn:
  - task-216
  - task-217
updatedAt: "2026-10-19T22:15:03.066Z"
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: a file from a work thread lands in the chat stamped from it; a quiet run's file is its one post
  validation:
    commands:
      - make check
      - "! git grep -n 'const QUIET:' -- crates/hotline-core/src/session/files.rs"
  metrics:
    readyAt: "2026-10-19T20:03:14.159Z"
    pickedUpAt: "2026-10-19T22:15:03.066Z"
    reworkCount: 1
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T22:14:53.718Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T22:14:55.793Z"
---

## Description
ADR 0008 §6 slice 3: send_file and generate_image from a work thread, a pair or a call post to the home thread through Room::post (task-216), stamped with where they came from; a quiet run's file is its post and counts as the turn's one post under the post_to_chat gate task-217 builds (use that gate; do not add a second count). Removes the quiet refusal (the QUIET constant) and its kept-copy cleanup in session/files.rs. Extend the headless harness that covers sent files: a file from a work thread lands in the chat with its `from` stamp and the thread keeps a line naming it; a quiet run's file is its post, and a post_to_chat after it in the same turn is refused.

From task-217 (landed on release/0.41.0): the posting gate lives in session/post.rs. Each turn opens it with Room::open_posting in dm.rs (the DM, carrying its job when quiet), turns.rs (Threads::turn: work threads and runs) and peers/delivery.rs (the pair's exchange turn, ThreadId::pair). A file counts against the Posting that is already open for the turn; do not open a second one. Calls don't open a Posting. The voice front's own post_to_chat in voice/front_tools.rs writes through room.post with no gate. So a file from a call lands stamped, with no once-a-turn count, and folding calls into the gate is out of scope here. Refusals for subagent runs and revoked leases are proved in session/tests/posting.rs. A file a subagent run sends should be refused there in the same way.

## Log
- 2026-10-19T22:14:52.782Z: [claude] Lead: try 1 ended its turn while `make check` was still running in the background, so nothing was validated or committed. Its work is uncommitted in the worktree on dispatch/task-219 (mcp/server.rs, session/files.rs, generate.rs, post.rs, sides.rs, tests/posting.rs, tests/sent_files.rs, tests/mcp/post_to_chat.rs, and design.md, security.md, sessions.md under docs). Next try: keep that work. Run `make check` in the foreground with a generous timeout (never in the background) and wait for it to finish. Fix what fails and commit, then run every contract validation command before delivering. Never end the turn while a command is still running.
