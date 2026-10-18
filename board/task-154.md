---
id: task-154
title: "P2-45a (D): Shared test plumbing: one bounded test Client in tests/common"
column: review
position: 43
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:03.758Z"
updatedAt: "2026-10-18T06:39:35.371Z"
relatedFiles:
  - crates/hotline-core/tests/common/mod.rs
  - crates/hotline-core/tests/
  - crates/hotline-cli/tests/
contract:
  status: delivered
  validation:
    commands:
      - test -d crates/hotline-cli/tests/common
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T06:19:02.655Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T06:39:35.371Z"
    duration: 1233
dependsOn:
  - task-132
  - task-133
  - task-141
  - task-147
  - task-152
  - task-153
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:15.500Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T06:18:53.191Z"
assignee: claude
---

## Description
Phase 2 group D, the first half of plan row 45 in docs/audits/0.40/README.md on release/0.40.0. Read tests-and-docs.md T4, T9, T14 and the Standards first. The `until(what, cond)` helper and the T8 sleep-then-assert sites are task-175, after this one.

One bounded test `Client` in `crates/hotline-core/tests/common/mod.rs`, then one file per commit for the seven copies. The CLI tests get a `crates/hotline-cli/tests/common` of their own. `tests/common` allows stay `#[allow(…, reason)]`, never `expect` (C5: each test binary uses a different subset).

End with `make precheck` and `make verify` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T06:39:34.996Z: [claude] Done on dispatch/task-154 (006de269), 12 commits. crates/hotline-core/tests/common/mod.rs now has the one Client. `call(cmd, params)` gets its reply within `common::ANSWER` (60 s) or the test panics. `send` plus `reply(id, patience)` is for tests where the wait itself is the proof: acp_startup's QUICK calls, and real_agents' two real-ACP session.start calls, which wait StartBounds::default().first + ANSWER. `subscribe` waits for and asserts ok, `next_where` names the last 12 inbox frames when it times out (taken from threads.rs), and `replied` is kept. stalled.rs lost its Client. desk, folders, steering, processes, threads, mcp and schedule each moved over in their own commit. local_speech.rs was an eighth copy the audit missed; it moved too (ask→call, models_until is now a free fn). New crates/hotline-cli/tests/common/mod.rs: hotline() (picks a free loopback --listen for serve), serve(root, extra), stop, wire(root, cmd, params), said, free_address, listen_args, top, desktop. serve.rs, relay.rs (a local serve_at wrapper) and pair.rs (hotline, free_address, a wire wrapper) each moved in their own commit. Every common allow is #[allow(dead_code, reason)]. make check, make precheck (7/7) and make verify pass; contract.ts unchanged. Not wired, for task-175 and later: tests/voice.rs keeps its own socket `until` (15 s, drops frames it doesn't match), and its seven inline Door::bind sites aren't converted to Client. No desk-and-door helper, no shared fake model server, and the per-file TOKEN consts stay (T14). The CLI's `invite`/`hello` differ between serve.rs and relay.rs, so each kept its own. Child processes still use the free_address reserve-and-drop race (task-147's note).
