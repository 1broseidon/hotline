---
id: task-136
title: "P2-27 (C): `driver/acp.rs` code"
column: review
position: 25
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:54.356Z"
relatedFiles:
  - crates/hotline-core/src/driver/acp.rs
  - crates/hotline-core/src/driver/acp/oauth_proxy.rs
  - crates/hotline-core/src/driver/acp/live.rs
  - crates/hotline-core/src/driver/acp/translate.rs
  - crates/hotline-core/src/driver/acp/session.rs
updatedAt: "2026-10-18T04:54:58.827Z"
contract:
  status: delivered
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/driver/acp.rs) -le 1500
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:36:14.227Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T04:54:58.827Z"
    duration: 1125
dependsOn:
  - task-135
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:55.256Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:36:06.317Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 27 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`driver/acp.rs` code** → `oauth_proxy.rs`, `live.rs`, `translate.rs`, `session.rs`

- Source: structure F2, task 13
- Files: `driver/acp.rs`
- Collides with: 3 (`acp/install.rs` is untouched, but land 3 first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T04:55:03.082Z: [claude] Done on branch dispatch/task-136 (commit bf4110d1, one move-only commit): driver/acp.rs 3182 → 990 lines; code moved unchanged into acp/oauth_proxy.rs (312), live.rs (430), translate.rs (~550, receive…usage_of, steer_lane, operator_*, text_block, file_uri) and session.rs (~920, second impl ChildAgent, connect, Raw*/Announced, initialize_request, granted_directories, capabilities_of, Disposition, apply_configs_to_session, choices_of). Per the plan, acp.rs keeps the process plumbing (pump_stderr, read/write_text_file, fail), lock and new_id. Children open with `use super::*` like auth.rs; acp.rs imports what it uses from them, and items used only by tests are imported in acp/tests.rs. Moved items/fields gained pub(super) only where the compiler needed it. Doc pointers updated: sessions.md (session/update → acp/session.rs, park_for_subagents → acp/live.rs), security.md:73 callback_workspace → acp/session.rs, mcp/server.rs comment. Left as is: the security.md test rows that still say `driver/acp.rs` for tests that live in acp/tests/* since task-135 (it's the same module). contract.ts byte-identical; make check and make precheck pass (7/7).
