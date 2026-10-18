---
id: task-146
title: "P2-37 (D): One `ANSWER` for the remote tests,"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:40:59.590Z"
relatedFiles:
  - crates/hotline-core/src/remote/tests.rs
  - crates/hotline-core/src/remote/tests/
updatedAt: "2026-10-18T05:21:25.359Z"
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T04:59:23.667Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T05:12:38.655Z"
    duration: 795
dependsOn:
  - task-134
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:06.499Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T04:59:15.620Z"
    ended: "2026-10-18T05:21:25.359Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T05:21:24.971Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 37 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One `ANSWER` for the remote tests,** at every wait for something that must arrive

- Source: tests-and-docs T2, task 2
- Files: `remote/tests.rs`, `remote/tests/*.rs`
- Collides with: 15, 25 (land first)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:12:38.179Z: [claude] Done on dispatch/task-146 (4dacba6e). ANSWER (15 s) now lives in remote/tests.rs beside a second constant, PROMPTLY (3 s), and every remote topic file uses them. ANSWER covers every wait for something that must arrive. PROMPTLY is for waits where a timer in the code would bring about the same result: the 5 s pre-auth lifetime and the relay's 10 s ACCEPT_WITHIN/IO_WITHIN. With 15 s there those tests would pass on the timer, so I kept them under it rather than raising them as the audit's list said. They are admission's counts/remote_counts and relay_url, the 'must yield capacity' waits (:314, :462, :484, :498), and relay's closed, callback_refused, hub_counts and the retire wait at :518. Left alone on purpose: admission :381 (1 s, proves eviction before expiry), the 7 s elapsed-ceiling/expiry waits in admission (T-other), addresses' HTTP timeouts (T1, row 38's loopback work), link's 30 s until, rust_client's 600 ms 'nothing arrives' checks, viewer_token's 30 s sleep (T5). Outside remote/ (driver/rig/turn.rs, voice/speech/xai/tests.rs, wire/tests.rs) the same short arrival waits remain, not in this row's scope. Remote lib tests 115 pass; make check and make precheck pass.
- 2026-10-18T05:21:24.579Z: Landed on release/0.40.0
