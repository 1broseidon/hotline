---
id: task-238
title: "S8: the whole on 0.45 — a 0.42 profile upgrades end to end over the wire"
column: in-progress
position: 7
priority: high
tags:
  - rig
  - auth
  - verify
relatedFiles:
  - crates/hotline-core/tests/rig_upgrade.rs
  - docs/plans/rig-0.45-migration.md
  - docs/releases
parentId: epic-22
dependsOn:
  - task-234
  - task-236
  - task-237
contract:
  status: in_progress
  deliverables:
    - type: test
      path: crates/hotline-core/tests/rig_upgrade.rs
      description: rig_upgrade_* harness over the wire
    - type: docs
      path: docs/plans/rig-0.45-migration.md
      description: what landed, plus George's pre-release manual checklist
  validation:
    commands:
      - test -f crates/hotline-core/tests/rig_upgrade.rs
      - grep -qE '^rig = \{ version = "=0\.45\.0"' Cargo.toml
      - make check
      - make verify
      - make precheck
      - git diff --exit-code HEAD -- ui/src/generated/contract.ts
      - git diff --exit-code $(git merge-base HEAD origin/release/0.42.0) HEAD -- ui/src/generated/contract.ts
      - test "$(cargo test -p hotline-core --test rig_upgrade -- --list 2>/dev/null | grep -c 'rig_upgrade_')" -ge 3
      - grep -qi 'checklist' docs/plans/rig-0.45-migration.md
  constraints:
    - "No wire type changes: ui/src/generated/contract.ts byte-identical to the base"
    - "Never touch real data: the harness uses its own temp HOTLINE_DATA_DIR and a test vault backend"
    - rig_pin_ assertions and expected values unchanged
  metrics:
    readyAt: "2026-10-20T12:04:00.313Z"
    pickedUpAt: "2026-10-20T14:02:08.955Z"
    reworkCount: 0
createdAt: "2026-10-20T12:04:00.326Z"
updatedAt: "2026-10-20T14:13:25.123Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T13:37:08.942Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T14:01:46.267Z"
assignee: codex
decisions:
  - decision-4
---

## Description
The closing proof for epic-22: the pieces S1, S4, S6 and S7 built, tested together against the epic's description of done. Add a headless harness `crates/hotline-core/tests/rig_upgrade.rs` (house idiom: start the real core, drive it over the wire, `make verify`) whose tests are prefixed `rig_upgrade_`:
- a data dir laid out by 0.42 (ChatGPT plaintext `auth.json`; Copilot `access-token` + `api-key.json`) starts on 0.45 and both sign-ins serve a turn against stub provider servers with no re-sign-in; afterwards the tokens live in the vault (test backend) and no plaintext token is left on disk;
- a multi-round Anthropic-shaped turn with cache reads/writes from a stub reports the same budget and context numbers as S0's `rig_pin_` baseline (cache tokens counted once) and every round's request carries the system prompt;
- a voice call against a stub settles its metering once per dispatch (S6's hook), including a dropped stream.
Fix what the whole shows broken in the slices' seams, in this task. Then bring docs/plans/rig-0.45-migration.md to what landed (one-jump path, what differs from the report, what was not adopted and why) and end it with the manual checklist George runs before cutting 0.42.0: one real turn each on Anthropic, OpenAI, ChatGPT and Copilot (existing sign-ins, chat and Responses), OpenRouter `anthropic/`, Ollama, a custom server, xAI subscription; an Anthropic multi-round turn shows cache_read > 0 on round 2; voice call metering; the macOS keychain migration of a 0.42-signed-in profile. Collect the slices' notes for George in that checklist. Confirm docs/releases carries the downgrade note from S4.

From S1 (task-231), for the doc and checklist. S1's manual checks: Anthropic key (2nd round shows cache_read > 0); OpenAI key; ChatGPT on an existing 0.42 sign-in (no re-sign-in) plus image generation and a GPT-Live call; Copilot on an existing 0.42 sign-in, chat route and a Responses-only model; OpenRouter anthropic/ (cached) and openai/; Gemini with an effort; Ollama local with an effort (think); a custom server on Responses and on Chat; xAI subscription and xAI key; z.ai; a voice call's metering against 0.42 numbers; fresh ChatGPT and Copilot device sign-ins. S1's deviations from the report, to record in the doc: OpenRouter marks Claude's system prompt through `Outgoing::mended` as well as the request's top-level `.cache(Short)` (S0's pin needs both); the Anthropic cache fold is deleted and an all-zero usage still counts as unreported; every request accepts unknown finish reasons (decision-3); the compat.rs decode probe is deleted; turn tests run on Rig's MockScript wire; ChatGPT/Copilot requests are signed per request in `providers/outgoing.rs`. Note that the 0.42 Copilot layout passes through S1's lazy migration to plaintext `copilot.json` and then S4's keychain move: the harness must prove both hops from a 0.42 dir.

## Log
- 2026-10-20T13:37:06.997Z: [claude] George, 2026-10-20 08:35: goes to Codex on gpt-6-astra. Run make check in the foreground and wait for it, commit before ending your turn, merge release/0.42.0 into the branch and re-run make check before saying done.
- 2026-10-20T14:02:15.039Z: [codex] Picked up S8 on dispatch/task-238. Inspecting the landed slice fixtures and real-wire harness before adding upgrade proofs; will run foreground make check, merge release/0.42.0, and rerun checks before handoff.
- 2026-10-20T14:10:58.163Z: [codex] Four real-wire rig_upgrade_ proofs pass: 0.42 ChatGPT plus both Copilot routes migrate to MemoryStore, cache usage/pricing and per-round system prompt match S0, cached context recovers, voice completion settles and truncated stream stays charged. Full desk tool definitions exceed S0's tiny context window, so that wire proof scales its usage/limits tenfold; original pins unchanged. Added dev-only credential-scoped loopback origins; updated landed migration record/checklist and stale design vault wording. Running full gates next.
- 2026-10-20T14:16:41.080Z: [codex] Committed 3d08b0c3 on dispatch/task-238 after foreground make check passed. Requested merge of release/0.42.0 reports already up to date. Running post-merge make check, make verify and make precheck; generated contract and S0 pin assertions remain unchanged. No shipping seam defect found; test-only origins and docs are wired, with real-provider/macOS checks explicitly left to George's checklist.
