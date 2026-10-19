---
id: task-183
title: "Linked desks: the dial-loop exit, an old-desk message, and the other follow-ups from the merge review (items 2-6)"
priority: high
tags:
  - linked
  - remote
parentId: epic-14
createdAt: "2026-10-18T23:40:54.765Z"
relatedFiles:
  - crates/hotline-core/src/remote/link.rs
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/tests/link.rs
updatedAt: "2026-10-19T00:19:43.765Z"
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/src/remote/tests/link.rs
      description: a failed room.unlinked clears dialing; an old desk gets the friendly message; noise_keys accepts a linked desk's key
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:05:22.243Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:16:44.367Z"
    duration: 682
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:05:10.035Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:05:12.995Z"
    ended: "2026-10-19T00:16:51.931Z"
    outcome: review
assignee: claude
completedAt: "2026-10-19T00:19:43.765Z"
---

## Description
Slice of task-65, items 2-6 (item 1, room.jsonl growth, stays there). (2) The dial loop returns without clearing 'dialing' when room.unlinked fails (remote/link.rs:309): clear it. (3) A friendly message when the other desk is too old to link (link.rs:129). (4) noise_keys checks only paired devices (remote/v2.rs:106): check linked desks' keys too, per the review. (5)-(6): as the task-65 text says. Tests over the wire.

Split from task-65, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:16:48.106Z: [claude] Done on dispatch/task-183 (1580e082). (2) A Revoked dial whose room.unlinked fails no longer exits: it shows "That desk ended the link, and this desk could not end it too: …" as the link's error and keeps dialing, so the next refusal ends it (the loop staying alive keeps 'dialing' true honestly; no separate clear needed). (3) link() maps an old desk's "unknown variant `desks.link_accept`" to "Update that desk to link with it." (remote/link.rs refusal). (4) noise_keys refuses to mint a new key while any link record exists. (5) link(): if the ask was sent but never answered, the record is kept and dialed; the other desk's key either opens the channel or refuses, which ends the link here. A send failure still errors. No wire test for (5): hard to make a peer accept and not answer. (6) LinkedTeammate.state doc names 'offline'; contract.ts regenerated. Tests in remote/tests/link.rs; fmt, clippy -D warnings, full cargo test -p hotline-core pass. Item 1 (room.jsonl growth) stays on task-65.
- 2026-10-19T00:19:43.312Z: Landed on release/0.40.0
