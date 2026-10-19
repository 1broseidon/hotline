---
id: task-188
title: Say 'Hotline Agent unavailable' when no provider is connected
column: in-progress
position: 14
priority: high
tags:
  - providers
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:57.376Z"
relatedFiles:
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/failure.rs
  - ui/src/components/settings
updatedAt: "2026-10-19T00:08:42.241Z"
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core
      description: "a desk with no provider connected says Hotline Agent unavailable: connect a provider, before the first turn, not as a failed turn"
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:08:42.241Z"
    reworkCount: 0
dependsOn:
  - task-8
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:08:30.763Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:08:33.017Z"
assignee: claude
---

## Description
Slice of task-97, item 1. When no provider is connected, Hotline Agent teammates should say why in a few words ('Hotline Agent unavailable: connect a provider'), with a link to Settings > Providers, not fail on the first turn. The desk.status view and the reachability row stay in task-97.

Split from task-97, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:08:29.452Z: [claude] Lead, from task-8 (landed c3d30f8c): driver/failure.rs now has Kind::ProviderRefused ("provider_refused"), plus Failure::checked_against_sign_in in driver/rig.rs, which probes a sign-in's model list when a turn gets a 401/403. Telling the person no provider is connected must happen before any turn, so it must not reach that probe or read as Auth or ProviderRefused. Follow set_kind's pattern for the wording, and the ErrorCard.tsx pattern for the Settings › Providers link.
