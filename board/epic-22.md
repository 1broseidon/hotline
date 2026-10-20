---
id: epic-22
title: "Rig 0.42 → 0.45: one version at a time, then sign-ins in the vault (0.42)"
type: epic
column: todo
position: 1
priority: high
tags:
  - rig
  - dependencies
  - "0.42"
assignee: claude
createdAt: "2026-10-20T06:51:41.804Z"
landsOn: release/0.42.0
updatedAt: "2026-10-20T12:00:12.786Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-20T12:00:11.276Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T12:00:12.786Z"
---

## Description
George, 2026-10-20 01:05 (phone): research Rig's migration docs, line them up with our codebase, and get it ready for release/0.42.0. Rig 0.45 (2026-10-19) ships George's own request, pluggable OAuth token storage (rig issue #2481, PRs #2815 and #2845), so ChatGPT and Copilot sign-ins can live in Hotline's vault instead of Rig's token files.

The research is in the workspace report rig-045-migration.md (copied into docs/plans/rig-0.45-migration.md by S0): sources (MIGRATING.md 0.42→0.43 and 0.43→0.44; the 0.44→0.45 guide landed after the tag in PR #2866), Hotline's whole Rig surface by file and line (about 40 sites in hotline-core and hotline-tools: the Hotline Agent driver, providers and five custom HTTP transports, seven tools, the voice dispatcher's hooks), every breaking change mapped to those lines against the 0.45 source, the token-store design, and the slices below.

Path: 0.42 → 0.43 → 0.44 → 0.45, one bump per slice, each green under make check and make precheck, contract.ts byte-identical throughout (no wire type changes). Pin rig =0.45.0 until 0.45.x settles; the rust-version rises to 1.97.1 in S3.

Silent changes the slices must handle: from 0.43 Anthropic input tokens already include cache reads and writes, and Hotline adds them again (turn.rs every_input_token, the dispatcher's cache_beside_input), so both budgets over-charge and the context limit trips early; 0.43 puts the system prompt in the chat history and turn.rs overwrites the history every round, so the prompt would vanish; 0.45's own file store reads chatgpt.json/copilot.json, so Hotline must ship its own TokenStore in the same slice or every ChatGPT and Copilot user is signed out.

Not adopted: Rig 0.45's resumable runs and committed-history streaming for the Hotline Agent turn loop. Hotline writes into history mid-turn and never re-runs a tool after a restart; Rig re-runs unanswered calls and its saved-run format changes between releases. The exactly-once on_dispatch/on_outcome hook is adopted for voice metering (S6).

Runs before epic-21 (voice on the agent thread layer) touches the voice dispatcher and front: S1 rewrites the dispatcher's hooks. About 11 to 16 days; S1 and S3 are the critical path.

Open for George: adopt ChatGPT's browser sign-in (PKCE on 127.0.0.1:1455) now that Rig supports it; revoke tokens at the provider on credential delete (S5).

## Log
- 2026-10-20T12:00:00.877Z: [claude] George, 2026-10-20 06:59 (call): go now, straight to 0.45, agents running. Order: S0 (task-230) pins today's behaviour on 0.42 → task-231 the single jump to 0.45 with the TokenStore → S4 keychain, S6 voice metering, S7 cleanups → S5 revoke. George QAs ChatGPT and Copilot sign-in continuity (and a quick Anthropic turn); Pork Chop covers xAI and the scripted checks on the QA computer. His two open questions (browser sign-in, revoke on delete) stay open; S5 waits for his answer on revoke.
- 2026-10-20T12:04:13.737Z: [claude (lead)] PLAN (lead, 2026-10-20). Order: task-230 S0 (seam + rig_pin_ tests on 0.42, copies report to docs/plans) → task-231 S1 (one jump to rig =0.45.0, rust 1.97.1, with LoginDirStore; S2/S3 were retired, so their whole scope is now written into S1's description) → task-234 S4 (vault), task-236 S6 (voice metering), task-237 S7 (cleanups) in parallel → task-238 S8 (new: rig_upgrade_ harness over the wire proving a 0.42 profile upgrades end to end, budgets equal the pins, voice settles once; brings the plan doc to what landed plus George's manual pre-release checklist). Why: S1 can't be split, a dependency bump compiles only whole, and sign-in continuity must ship with it. S4/S6/S7 touch disjoint code (rig_login.rs+vault.rs+failure.rs / voice/dispatcher.rs / copilot.rs+compat.rs+build.rs+turn.rs); the doc conflict the app flagged is gone because only S0 and S8 edit docs/plans/rig-0.45-migration.md. task-235 S5 now waits on 234 and 237 (it touches copilot.rs too) AND on George's answer to 'revoke at the provider on delete': it is not handed off until he answers here; if he says yes, add task-235 to task-238's dependsOn before S8 starts. Every child has a contract: make check, make precheck, contract.ts identical to its merge-base, and a count of its named tests (rig_pin_/rig_login_/rig_vault_/voice_meter_/rig_revoke_/rig_upgrade_). Manual real-provider turns can't happen before landing in bypass mode; they are George's checklist in S8. Agents: claude opus for all; S1 and S8 at xhigh.
