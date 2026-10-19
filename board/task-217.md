---
id: task-217
title: "Slice 2: post_to_chat on the desk's MCP server for both drivers; tell_person and Escalate go"
column: review
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
updatedAt: "2026-10-19T21:33:26.138Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T21:16:30.272Z"
    reworkCount: 2
    deliveredAt: "2026-10-19T21:33:26.138Z"
    duration: 1016
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T20:51:29.852Z"
  until: done
  tries: 3
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T21:16:20.771Z"
---

## Description
ADR 0008 §3 and §6 slice 2: post_to_chat(text, wake?) on Hotline's MCP server, same handler in-process for Hotline Agent; the gate (kind, modality, once a turn, one wake a turn, lease, 2,000 chars); a subagent the home agent launched is refused. security.md: default, grant source, enforcement, tests (every refusal row in the ADR's table on both drivers), residual risk. Removes the Escalate trait, escalate_next, the Rig-only tell_person and per-turn arming. An ACP quiet run that posts is proved.

From task-216 (landed on release/0.41.0): Room::post lives in crates/hotline-core/src/session/post.rs, and call_posted, the quiet run's follow-up and send_file from a thread already write through it. post_to_chat and the Escalate path were left alone and still reach the chat their old way: move them onto Room::post (stamp `from` with the thread, `scheduled` where a job asked), so no second door to the chat is left. task-218's pairs build through Room::thread_agent, so wire a pair's thread kind for the gate there, not in peer_session.

## Log
- 2026-10-19T20:51:28.807Z: [claude] Lead: try 1 ended its turn while `make check` was still running, so nothing was validated. Its work is on dispatch/task-217 (4c256848, 4a68b1d6) with uncommitted edits to session/post.rs and session/prompt.rs. Next try: keep that work, run `make check` in the foreground (generous timeout, never backgrounded) and wait for it, fix what fails, commit, then run every contract validation command before delivering. Do not end the turn while a command is pending.
- 2026-10-19T21:01:12.855Z: [claude] Try 2 kept try 1's work (4c256848, 4a68b1d6) and committed c8fa7e59. That commit holds try 1's uncommitted posted_since refactor (one sentence for posts from elsewhere, shared by prompt.rs and post.rs) plus fixes for what make check caught: two clippy lints (assert_is_empty in session/tests/posting.rs, equatable_if_let in tests/mcp/post_to_chat.rs), and a race in the quiet-run harness. That harness scheduled its job exactly 1s ahead, the server's minimum, so wire latency got it refused; it now uses 1,500ms like tests/schedule.rs. make check passes, make precheck passes all 8 legs, and every contract validation passes (the escalate/tell_person grep is empty; post_to_chat appears in mcp/ and security.md; tests/mcp passes 19/19). For later slices: the posting gate opens per turn through Room::open_posting in dm.rs (DM, with its job when quiet), turns.rs (Threads::turn: work threads and runs) and peers/delivery.rs (the pair's exchange turn, ThreadId::pair), not in peer_session. Calls don't open a Posting; a voice front's post_to_chat is still its own in voice/front_tools.rs. Moving calls onto Opening/Threads::drive is the place to fold that in. The subagent-run refusal and revoked lease are proved in session/tests/posting.rs (lease in process and over the served endpoint), because subagent runs exist only on Hotline Agent. Work is on dispatch/task-217.
- 2026-10-19T21:16:20.215Z: Landing sent back (1 of 1): with release/0.41.0 brought in, `make check` failed: stopped after 15 minutes. Fix it on this branch, commit, and run the checks again.
- 2026-10-19T21:33:30.294Z: [claude] Landing fix: make check's 15-minute stop was a hang in task-202's wire::tests::roster::the_lead_is_set_moved_and_cleared_and_the_roster_rows_carry_it, not this slice. The roster re-sends the two rows from a HashSet (wire/roster.rs Dirty), so they come in random order; the test waited for Bob's row before Ada's, skipped Ada's on the way, then blocked forever on a heard() with no timeout. It fails about half the time on release/0.41.0 too. Fixed in 572a84bb: the test now collects both rows in either order (10/10 runs pass). All of make check's steps pass (cargo test --workspace exit 0, fmt, clippy, ui-check, deps, python), make precheck passes all 8 legs, and every contract validation passes (escalate/tell_person grep empty, post_to_chat in mcp/ and security.md, tests/mcp 19/19). The slice's handoff from try 2 still holds: calls don't open a Posting, and the voice front's post_to_chat is still its own in voice/front_tools.rs. On dispatch/task-217.
