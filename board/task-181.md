---
id: task-181
title: "Agent registry: keep the last working list, refresh it in the background at session start, and say plainly when a model isn't supported"
column: todo
position: 7
priority: high
tags:
  - acp
  - registry
  - bugfix
parentId: epic-14
createdAt: "2026-10-18T23:40:53.687Z"
relatedFiles:
  - crates/hotline-core/src/driver/acp/registry.rs
  - crates/hotline-core/src/desk.rs
  - crates/hotline-core/src/wire/room_handle.rs
  - crates/hotline-core/src/driver/failure.rs
updatedAt: "2026-10-19T00:27:31.124Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/src/driver/acp/registry.rs
      description: a failed fetch keeps the last good list; a stale cache refreshes in the background without blocking session start; model-not-supported reads as a too-old harness with what to do
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - cargo test -p hotline-core --test acp_startup
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dependsOn:
  - task-188
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:27:29.287Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:27:31.124Z"
assignee: claude
---

## Description
Slice of task-14 (items 2-4). driver/acp/registry.rs: 24 h TTL (:36), only the picker refreshes (backends(), desk.rs), session start uses cached_backends, and the hand-pinned codex-acp fallback sits at :198. (a) A failed registry fetch keeps the last known good list. (b) Session start refreshes in the background when the cache is stale, without blocking. (c) A 'model not supported by this harness version' failure becomes a card saying which harness is too old and what to do, not a raw 400. Leave auto-refresh-and-retry for later. Tests for each.

Split from task-14, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:08:29.760Z: [claude] Lead, from task-8 (landed): failure.rs has a new Kind::ProviderRefused and probes the provider's model list on a 401/403. A harness that says 'model not supported' (a 400) is neither of those: give it its own wording in set_kind, naming the harness and saying to update it. Don't fold it into Auth, InvalidRequest or ProviderRefused. task-188 also changes failure.rs before you start, so rebase onto release/0.40.0.
