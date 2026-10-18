---
id: task-175
title: "P2-45b (D): One until(what, cond) with one deadline, replacing the T8 sleep-then-assert sites"
column: todo
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
updatedAt: "2026-10-18T06:43:44.165Z"
contract:
  status: ready
  validation:
    commands:
      - git grep -q 'fn until' -- crates/hotline-core/tests/common
      - make precheck
      - make check
      - make verify
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
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
