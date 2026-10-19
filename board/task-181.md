---
id: task-181
title: "Agent registry: keep the last working list, refresh it in the background at session start, and say plainly when a model isn't supported"
column: review
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
updatedAt: "2026-10-19T00:40:29.320Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T00:37:35.904Z"
    reworkCount: 1
    deliveredAt: "2026-10-19T00:40:29.320Z"
    duration: 173
dependsOn:
  - task-188
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:37:19.220Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:37:24.565Z"
assignee: claude
---

## Description
Slice of task-14 (items 2-4). driver/acp/registry.rs: 24 h TTL (:36), only the picker refreshes (backends(), desk.rs), session start uses cached_backends, and the hand-pinned codex-acp fallback sits at :198. (a) A failed registry fetch keeps the last known good list. (b) Session start refreshes in the background when the cache is stale, without blocking. (c) A 'model not supported by this harness version' failure becomes a card saying which harness is too old and what to do, not a raw 400. Leave auto-refresh-and-retry for later. Tests for each.

Split from task-14, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:08:29.760Z: [claude] Lead, from task-8 (landed): failure.rs has a new Kind::ProviderRefused and probes the provider's model list on a 401/403. A harness that says 'model not supported' (a 400) is neither of those: give it its own wording in set_kind, naming the harness and saying to update it. Don't fold it into Auth, InvalidRequest or ProviderRefused. task-188 also changes failure.rs before you start, so rebase onto release/0.40.0.
- 2026-10-19T00:37:17.544Z: [claude] Lead: the last try ended its turn while the full hotline-core test run was still going in the background. It committed nothing, so its work sits uncommitted in the worktree: driver/acp.rs, acp/auth.rs, acp/auth/tests.rs, acp/registry.rs, failure.rs and contract.ts. Start from those changes; don't redo them. Run every validation command in the foreground and wait for it to finish (`cargo test -p hotline-core` takes several minutes; give it a timeout of 10 minutes). Never end your turn while a command is still running. Commit, then deliver.
- 2026-10-19T00:40:28.961Z: [claude] Done on dispatch/task-181 (78799ce2). (a) refresh_catalogue writes only a non-empty agent list, via a temp file renamed over the cache, so a failed fetch (no answer, error status, bad body, empty list) keeps the last good copy. (b) registry::refresh_in_background, called by backends() and ChildAgent::start, spawns the fetch when the cache is stale and returns at once; the start runs on the copy it has. (c) New failure Kind::HarnessOutdated ("harness_outdated"), for acp phases only, matched on 'requires a newer version' / 'not supported by this (harness) version' / 'upgrade to the latest'; auth::failure names it on the card ('Codex 1.12.0 is too old for this model') from initialize's agent name and version; summary says to update the harness or switch model. Failure.title is now Cow<'static, str>. Tests: registry a_failed_fetch_keeps_the_last_good_list, a_stale_catalogue_is_refreshed_behind_a_session_start_without_holding_it_up; failure a_harness_too_old_for_its_model_says_to_update_it; auth a_harness_too_old_for_its_model_is_named_on_its_card. Left for later: auto-refresh-and-retry on HarnessOutdated, and no UI button for it (ErrorCard shows title/summary only). fmt, clippy, full hotline-core tests, acp_startup and contract.ts diff pass.
