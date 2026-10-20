---
id: task-237
title: "S7: cleanups 0.45 makes possible"
column: in-progress
position: 9
priority: low
tags:
  - rig
assignee: codex
relatedFiles:
  - crates/hotline-core/src/providers/outgoing.rs
  - crates/hotline-core/src/providers/copilot.rs
  - crates/hotline-core/src/providers/compat.rs
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/rig/turn.rs
parentId: epic-22
createdAt: "2026-10-20T06:51:46.152Z"
dependsOn:
  - task-231
updatedAt: "2026-10-20T13:37:13.904Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-core/src/providers/copilot.rs
      description: ChatBody removed if the pin allows; /responses via with_route
  validation:
    commands:
      - make check
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test --workspace -- --list 2>/dev/null | grep -c 'rig_pin_')" -ge 6
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - rig_pin_ assertions and expected values unchanged
    - Do not touch voice/dispatcher.rs, providers/rig_login.rs or docs/plans/rig-0.45-migration.md
  metrics:
    readyAt: "2026-10-20T12:03:23.766Z"
    pickedUpAt: "2026-10-20T13:31:43.244Z"
    reworkCount: 0
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T13:37:08.682Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T13:37:13.904Z"
---

## Description
Report §6 S7 (docs/plans/rig-0.45-migration.md). Cleanups 0.45 makes possible, each proved by S0's `rig_pin_` snapshots staying unchanged: delete Copilot's chat-text flatten if the Copilot string-content pin passes without it; Copilot `/responses` via `with_route(Route::Responses)` on the shared dialect; the remaining `compat.rs` repairs, deleted where 0.45 makes them unnecessary. Optional, only if it makes the code smaller and the pins stay green: typed `.reasoning(Effort)` with `OnUnsupported::Ignore` replacing `effort_params` (check Gemini's `thinkingLevel` casing, report §7.9); a `pairing::validate` debug assert before each request. Say in your final note which optional items you took and why. Do not touch `voice/dispatcher.rs` (S6), `providers/rig_login.rs`, `vault.rs` or `driver/failure.rs` (S4); do not edit docs/plans/rig-0.45-migration.md (the closing task does).

What S1 (task-231) changed, as landed on release/0.42.0: model requests for ChatGPT and Copilot now go through `providers/outgoing.rs` `Outgoing`, which calls `auth_context` per request; Copilot's chat-text flatten (formerly `ChatBody` in copilot.rs) now lives there as its body mend, so that is where to delete it. The compat.rs decode probe is already deleted (0.45 reads Responses frames as JSON); its repairs stay for you to judge. OpenRouter: 0.45's request `.cache(Short)` puts the marker only at the top level, but S0's pin wants it on Claude's system prompt, so `build::openrouter_model` also marks the prompt through `Outgoing::mended` — leave both unless the pin holds with one. Turn tests run on Rig's MockScript wire through a channel-fed Transport, which joins adjacent user messages and renders tool calls as text: compare parts or read the turn's kept history.

## Log
- 2026-10-20T13:37:06.491Z: [claude] George, 2026-10-20 08:35: the rest of epic-22 is mechanical and goes to Codex on gpt-6-astra. A Claude Opus run started this and was stopped after about 5 minutes; its worktree may hold partial uncommitted edits: review them, keep what's right, discard the rest. Run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done (a landing's check stops after 15 minutes).
