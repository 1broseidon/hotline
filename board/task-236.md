---
id: task-236
title: "S6: voice metering keyed by dispatch id (Rig 0.45's exactly-once outcome hook)"
column: in-progress
position: 8
priority: medium
tags:
  - rig
  - voice
  - budgets
assignee: codex
relatedFiles:
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/tests/failures.rs
parentId: epic-22
createdAt: "2026-10-20T06:51:45.659Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T13:37:33.656Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/dispatcher.rs
      description: one dispatch/outcome metering hook keyed by EffectId
    - type: test
      path: crates/hotline-core
      description: voice_meter_* tests
  validation:
    commands:
      - "! grep -n 'StreamMeter' crates/hotline-core/src/voice/dispatcher.rs"
      - make check
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'voice_meter_')" -ge 4
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - Do not edit docs/plans/rig-0.45-migration.md
  metrics:
    readyAt: "2026-10-20T12:03:23.226Z"
    pickedUpAt: "2026-10-20T13:37:33.656Z"
    reworkCount: 1
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T13:37:08.409Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T13:37:10.867Z"
---

## Description
Report §6 S6 and §5 (docs/plans/rig-0.45-migration.md). Builds on S1's minimal voice port in `voice/dispatcher.rs`. Replace `Meter`/`StreamMeter` with one hook: `on_dispatch` reserves (deny when over budget), `on_outcome` settles by `EffectId`; remove the single reservation slot. Record the metering rule in docs/voice-calls.md or docs/design.md, wherever voice budgets are described today.

Tests, prefixed `voice_meter_`: a provider error keeps its reservation; a retried turn settles both attempts; a stream dropped mid-reply stays charged; a budget refusal lands on the next dispatch. Do not edit docs/plans/rig-0.45-migration.md (the closing task does).

What S1 (task-231) left you, as landed on release/0.42.0: one `Meter` reserves in `on_completion_call` and settles in `on_outcome`, for streamed and unary runs; a failed completion keeps its reservation. The Anthropic cache fold is gone (`Tokens::of` takes no provider); an all-zero usage still counts as unreported (0.45's `is_reported` treats Some(0) as reported), so meters keep their reservations as on 0.42 — keep that rule. Every request accepts unknown finish reasons (decision-3). Touch only the voice dispatcher, its tests and the voice docs: S4 (rig_login.rs, vault.rs, failure.rs) and S7 (outgoing.rs, copilot.rs, compat.rs, build.rs, turn.rs) run alongside.

## Log
- 2026-10-20T13:37:05.985Z: [claude] George, 2026-10-20 08:35: the rest of epic-22 is mechanical and goes to Codex on gpt-6-astra. A Claude Opus run started this and was stopped after about 5 minutes; its worktree may hold partial uncommitted edits: review them, keep what's right, discard the rest. Run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done (a landing's check stops after 15 minutes).
