---
id: task-144
title: "P2-35 (D): Discovery client is fallible"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:58.554Z"
relatedFiles:
  - crates/hotline-core/src/providers/discovery.rs
updatedAt: "2026-10-18T02:48:19.191Z"
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:27:44.367Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T02:40:18.272Z"
    duration: 754
dependsOn:
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:04.262Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:27:32.580Z"
    ended: "2026-10-18T02:48:19.191Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T02:48:17.772Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 35 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Discovery client is fallible.** Remove `Default`, propagate the error, and keep the previous list

- Source: errors F2, task 2
- Files: `providers/discovery.rs:95,219,249,293`
- Collides with: 39 (`future_not_send` at `:182`)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T02:40:21.818Z: [claude] Done on dispatch/task-144, one commit (030aedc5). `impl Default for DiscoveryHttp` is gone. `DiscoveryHttp::new() -> Result<Self, String>` replaces it ("Could not prepare model discovery; the previous list is unchanged."), and `built(builder)` is the seam for a test. All seven call sites now use `new()?`: the report's three in discovery.rs, plus four it missed (copilot.rs:155, custom.rs:99, chatgpt.rs:103, xai.rs:456). The previous list is kept because desk.rs credential_refresh_models returns the error before cache_discovery. Unit test a_client_that_cannot_build_refuses_the_listing_instead_of_panicking makes the build fail with use_preconfigured_tls(()). NOT done: the report's wire-level proof with an injected failure. That would need a global failure hook in the core path, which is more machinery than an S row should add, so the unit test proves the constructor path instead. For row 39: `collect` (future_not_send) moved from :182 to about :191. make check and make precheck (7/7) pass.
- 2026-10-18T02:48:16.417Z: Landed on release/0.40.0
