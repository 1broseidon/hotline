---
id: task-175
title: "P2-45b (D): One until(what, cond) with one deadline, replacing the T8 sleep-then-assert sites"
column: review
position: 64
description: |-
  Phase 2 group D, the second half of plan row 45 in docs/audits/0.40/README.md on release/0.40.0 (split from task-154, which builds the shared Client and tests/common first). Read tests-and-docs.md T8, T9, T14 and the Standards ("Wait on a signal through one until(what, cond) with one generous deadline. A test asserts floors and order, never an elapsed-time ceiling").

  Add one `until(what, cond)` helper with one deadline to `crates/hotline-core/tests/common/mod.rs` (and the CLI's `tests/common` that task-154 adds), then replace the T8 sleep-then-assert sites module by module, one commit per module.

  End with `make precheck` and `make verify` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - fixes
  - tests
relatedFiles:
  - crates/hotline-core/tests/common/mod.rs
  - crates/hotline-core/tests/
  - crates/hotline-cli/tests/
parentId: epic-13
createdAt: "2026-10-17T22:45:36.667Z"
updatedAt: "2026-10-18T07:15:34.368Z"
contract:
  status: delivered
  validation:
    commands:
      - git grep -q 'fn until' -- crates/hotline-core/tests/common
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T06:44:11.270Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T07:15:34.368Z"
    duration: 1883
dependsOn:
  - task-154
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:36.738Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T06:43:44.165Z"
assignee: claude
---

## Description
Phase 2 group D, the second half of plan row 45 in docs/audits/0.40/README.md on release/0.40.0 (split from task-154, which builds the shared Client and tests/common first). Read tests-and-docs.md T8, T9, T14 and the Standards ("Wait on a signal through one until(what, cond) with one generous deadline. A test asserts floors and order, never an elapsed-time ceiling").

Add one `until(what, cond)` helper with one deadline to `crates/hotline-core/tests/common/mod.rs` (and the CLI's `tests/common` that task-154 adds), then replace the T8 sleep-then-assert sites module by module, one commit per module.

End with `make precheck` and `make verify` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T07:15:33.885Z: [claude] Done on dispatch/task-175 (228996dc), 21 commits. One until(what, cond) with one deadline, a minute, in three places: crate::test_support::until (new #[cfg(test)] module, PATIENCE 60 s) for unit tests, common::until (common::ANSWER) for core harnesses, and a sync common::until (common::PATIENCE) in the CLI's tests/common, which serve() and stop() now use. Each module moved in its own commit: the four old until helpers (remote link, session links, exchanges, threads, all now named), voice until_written, pair.rs eventually, and session tests' settled/heard/until_state/until_awaiting/heard_delivery plus peers' 100×1 ms collaboration_card. T8 sites: gate.rs (waits for the ended gate's release), jobs.rs (waits for jobs.ended to be non-empty instead of 500 ms), sides tests (named waits; the 5 ms timestamp gaps are now a_millisecond_passes on now_ms; the cancel waits for the prompt), voice clips (the 30 ms "let the worker start" removed: nothing runs at start), turn backoff tests (start_paused, so the 100 ms returns only once the turn is parked in its backoff), tests/voice after_call (hangs up once the front's heard frame arrives) and front_tools (waits for the turn event, not 300 ms). Also: tests/voice's socket until is renamed frame_where and waits common::ANSWER; stalled.rs's group_gone/process_gone became process_alive plus until; steering.rs drops its began.elapsed() < 15 s ceiling (the later "done never written" assert is the order proof); CLI relay's standing() moved to the CLI's common and serve.rs uses it. development.md § The harness says how a test waits. make check, make precheck (8/8) and make verify pass; contract.ts unchanged. Left for later: gate.rs 554/560 (600 ms vs 400 ms idle) are floors, not signals, so they stay; the sleep-then-assert-nothing sites (T15: sides 1022, voice direct.rs 146/271/381/444/498, processes 300 ms, steering 200 ms, voice processes 1 s, link.rs 500 ms) stay; about 90 inline `for _ in 0..N` poll loops in src tests (wire/tests/roster, remote/tests/admission, session/tests/*, driver/acp/tests/*) are not yet on until; async wire-retry loops (front.rs front_audio, processes.rs process.output/fetch, local_speech models_until, relay hello loop) can't take a sync condition and keep their own deadlines.
