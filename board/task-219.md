---
id: task-219
title: "Slice 3: files and images from any thread land in the chat"
column: todo
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
updatedAt: "2026-10-19T20:03:14.159Z"
contract:
  status: ready
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
---

## Description
ADR 0008 §6 slice 3: send_file and generate_image from a work thread, a pair or a call post to the home thread through Room::post (task-216), stamped with where they came from; a quiet run's file is its post and counts as the turn's one post under the post_to_chat gate task-217 builds (use that gate; do not add a second count). Removes the quiet refusal (the QUIET constant) and its kept-copy cleanup in session/files.rs. Extend the headless harness that covers sent files: a file from a work thread lands in the chat with its `from` stamp and the thread keeps a line naming it; a quiet run's file is its post, and a post_to_chat after it in the same turn is refused.
