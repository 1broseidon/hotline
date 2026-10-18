---
id: task-188
title: Say 'Hotline Agent unavailable' when no provider is connected
column: todo
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
updatedAt: "2026-10-18T23:53:55.552Z"
contract:
  status: ready
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
dependsOn:
  - task-8
---

## Description
Slice of task-97, item 1. When no provider is connected, Hotline Agent teammates should say why in a few words ('Hotline Agent unavailable: connect a provider'), with a link to Settings > Providers, not fail on the first turn. The desk.status view and the reachability row stay in task-97.

Split from task-97, which keeps the rest and stays deferred.
