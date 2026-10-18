---
id: task-151
title: "P2-42 (D): `result_large_err`: all 13 become `#[expect(…, reason)]`"
column: in-progress
position: 40
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:02.193Z"
relatedFiles:
  - crates/hotline-core/src/driver/rig/turn.rs
  - crates/hotline-core/src/driver/rig.rs
  - crates/hotline-core/src/wire/
  - crates/hotline-core/src/remote/bridge.rs
updatedAt: "2026-10-18T09:49:14.234Z"
contract:
  status: in_progress
  validation:
    commands:
      - "! git grep -n 'allow(clippy::result_large_err' -- crates"
      - test $(git grep -n 'expect(clippy::result_large_err' -- crates | wc -l) -ge 13
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T09:49:14.234Z"
    reworkCount: 0
dependsOn:
  - task-150
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:12.125Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T09:48:55.338Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 42 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`result_large_err`: all 13 become `#[expect(…, reason)]`.** Five name the structured `Failure`, eight name tungstenite's fixed callback type. Boxing `Failure` is dropped unless George wants it (C6)

- Source: errors F5, task 9; lints, task 6
- Files: `driver/rig/turn.rs:43,141,194,689`, `driver/rig.rs:718`, `wire` (was `mod.rs:1021`), `remote/bridge.rs:236`, six test sites
- Collides with: 31, 41
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T09:58:16.901Z: [claude] All 17 result_large_err allows are now #[expect(…, reason)], up from the plan's 13 because the task-31 split added turn.rs sites: 9 rig Failure sites (rig.rs run; turn.rs reserve, run, run_inner, stream_response, recover, retry, dispatch_calls, execute) and 8 tungstenite-callback sites (wire/door.rs, remote/bridge.rs, remote/tests.rs, voice realtime socket tests ×3, xai tests ×2). Every expectation fires under clippy --all-targets. Failure is not boxed (C6). No allow(clippy::result_large_err) is left for task 52's lint table. make check and make precheck pass. Commit 6ebd702a is on dispatch/task-151.
