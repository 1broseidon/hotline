---
id: task-183
title: "Linked desks: the dial-loop exit, an old-desk message, and the other follow-ups from the merge review (items 2-6)"
column: todo
position: 9
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
updatedAt: "2026-10-18T23:53:55.552Z"
contract:
  status: ready
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
---

## Description
Slice of task-65, items 2-6 (item 1, room.jsonl growth, stays there). (2) The dial loop returns without clearing 'dialing' when room.unlinked fails (remote/link.rs:309): clear it. (3) A friendly message when the other desk is too old to link (link.rs:129). (4) noise_keys checks only paired devices (remote/v2.rs:106): check linked desks' keys too, per the review. (5)-(6): as the task-65 text says. Tests over the wire.

Split from task-65, which keeps the rest and stays deferred.
