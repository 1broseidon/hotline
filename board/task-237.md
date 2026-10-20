---
id: task-237
title: "S7: cleanups 0.45 makes possible"
column: review
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
updatedAt: "2026-10-20T13:49:29.545Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-20T13:37:40.860Z"
    reworkCount: 1
    deliveredAt: "2026-10-20T13:49:29.545Z"
    duration: 709
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
- 2026-10-20T13:37:42.108Z: [codex] Resumed prior Copilot flatten commit and uncommitted compat deletion; reviewing pins and upstream implementation before keeping changes. MCP discovery unavailable, using authorized Brainfile CLI fallback.
- 2026-10-20T13:39:05.341Z: [codex] All 12 rig_pin_ tests pass (11 original assertions unchanged plus loose custom Responses two-round regression). Kept prior flatten deletion; Copilot shared DIALECT with_route(Responses) already landed in S1. Removed compat repairs: Rig JSON readers tolerate absent ids/status/sequence, null summaries/arguments, float timestamps, echoed instructions and loose metadata. OpenRouter markers retained. Optional typed reasoning skipped: Gemini uses correct thinkingLevel/lowercase but model support gating changes current raw effort behavior; pairing assert skipped because it removes no code. make check running foreground.
- 2026-10-20T13:45:06.371Z: [codex] Full make check passed after rustfmt corrected the inherited test formatting. Committing compat cleanup alongside the earlier Copilot flatten removal; requested merge of release/0.42.0 reports already up to date. Running make check again after merge, then precheck.
- 2026-10-20T13:49:29.080Z: [codex] Delivered on dispatch/task-237: e775f2f1 removes Copilot chat flatten; 6c69ae37 deletes compat.rs and uses native Reqwest transport, with a two-round loose Responses regression. Copilot shared-dialect with_route(Responses) was already wired by S1; nothing left unwired. All 12 rig_pin_ tests pass, original assertions/expected values unchanged. make check passed before and after the requested release/0.42.0 merge (already up to date); make precheck passed all 8 legs including Windows and isolated UI tests. Workspace test listing confirms 12 pins; both required contract.ts comparisons pass; working tree clean. Optional items: neither taken—typed reasoning adds mapping/support-gating differences (Gemini thinkingLevel spelling and lowercase values verified in Rig 0.45 source); pairing debug assert removes no code. OpenRouter cache markers retained. Reviewer can land this branch; closing task should update migration report, which was intentionally untouched.
