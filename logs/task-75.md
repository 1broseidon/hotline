---
id: task-75
title: "Desk: a call keeps its own transcript and posts summaries to the chat"
priority: high
tags:
  - voice
  - front
  - record
parentId: epic-7
contract:
  status: delivered
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/duplex.rs
      description: Summary at hang-up and on an interval, no per-turn mirror
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: DM holds the hand-off's turn plus one summary per call
  validation:
    commands:
      - make check
      - cargo test -p hotline-core --test voice
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness in crates/hotline-core/tests/; doc drift is a bug"
    - Run cargo test -p hotline-core UNFILTERED before committing ui/src/generated/contract.ts (a filtered run truncates it)
    - Capability changes follow docs/security.md (post_to_chat is a new front tool)
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T14:05:40.363Z"
    pickedUpAt: "2026-10-16T14:49:25.900Z"
    reworkCount: 0
    deliveredAt: "2026-10-16T14:58:40.993Z"
    duration: 555
createdAt: "2026-10-16T14:02:49.796Z"
dependsOn:
  - task-74
updatedAt: "2026-10-16T15:03:14.131Z"
relatedFiles:
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/front_tools.rs
  - crates/hotline-core/tests/voice.rs
dispatch:
  agent: claude
  model: sonnet
  by: lead
  at: "2026-10-16T14:49:02.532Z"
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T14:49:04.750Z"
    ended: "2026-10-16T14:58:47.166Z"
    outcome: review
assignee: claude
completedAt: "2026-10-16T15:03:14.131Z"
---

## Description
Stop mirroring each front turn into the teammate's DM. The call's own record (voice/record.rs, the call card's transcript) keeps what was said. At hang-up, and every 5 minutes on a longer call, post one summary to the DM: what was asked, what was done (tools run, work handed off), anything left open, tagged as from the call. A post_to_chat front tool posts on request ('put that in the chat'): add it to the registry in voice/front_tools.rs that task-74 built, run desk-side like the others. Work handed to the working session still runs in the open chapter as today. Keep the follow-up for a question the call ended on. Files: voice/duplex.rs (carry_front, which task-74 also changed), voice/spoken.rs, voice/record.rs, voice/mod.rs (Origin), voice/front_tools.rs. Harness in crates/hotline-core/tests/voice.rs: a call with three exchanges and one hand-off leaves exactly the hand-off's turn plus one summary in the DM.

## Log
- 2026-10-16T14:58:40.685Z: [claude] Done on branch dispatch/task-75, make check green. There was no per-turn mirror to remove: front turns were already only on the call thread, so the DM got only the hand-off turn. Added: record.rs Digest (asked / tools run / handed off / left open, built mechanically, no model call); duplex.rs posts one "From the call:" summary at hang-up and every 5 min (SUMMARY_EVERY, nothing if nothing happened; "Left open" only at hang-up; follow-up for the ended-on question unchanged); post_to_chat(text) in front_tools.rs (FrontTools::open now takes room, call id), checked against the lease, 2000-char clip. Posts are Room::call_posted: a tape `notice` (level info) with text prefixed "From the call:", no separate call tag field, so the UI shows them as plain notices. Harness: tests/voice.rs a_front_call_posts_one_summary_to_the_dm_and_not_its_turns (3 exchanges + 1 hand-off = one user turn + one notice) and post_to_chat/revoked cases in the desk-tool test. Unwired: Origin untouched; GPT-Live (task-79) must feed digest.ran/handed from its desk-side loop and get post_to_chat from FrontTools::definitions; the 5-min interval has no test (const, no clock injection); front_prompt.rs doesn't tell the model about post_to_chat (task-77); docs/security.md not updated for post_to_chat (lease-checked, tape write only).
- 2026-10-16T15:01:23.922Z: [claude] Reviewed by George's agent 2026-10-16: landed for the plumbing (no per-turn mirror, post_to_chat, 5-minute and hang-up summaries); the summary's wording, author and history are reworked in the follow-up child.
- 2026-10-16T15:03:13.723Z: Landed on release/0.39.0
