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
updatedAt: "2026-10-20T06:51:42.259Z"
---

## Description
George, 2026-10-20 01:05 (phone): research Rig's migration docs, line them up with our codebase, and get it ready for release/0.42.0. Rig 0.45 (2026-10-19) ships George's own request, pluggable OAuth token storage (rig issue #2481, PRs #2815 and #2845), so ChatGPT and Copilot sign-ins can live in Hotline's vault instead of Rig's token files.

The research is in the workspace report rig-045-migration.md (copied into docs/plans/rig-0.45-migration.md by S0): sources (MIGRATING.md 0.42→0.43 and 0.43→0.44; the 0.44→0.45 guide landed after the tag in PR #2866), Hotline's whole Rig surface by file and line (about 40 sites in hotline-core and hotline-tools: the Hotline Agent driver, providers and five custom HTTP transports, seven tools, the voice dispatcher's hooks), every breaking change mapped to those lines against the 0.45 source, the token-store design, and the slices below.

Path: 0.42 → 0.43 → 0.44 → 0.45, one bump per slice, each green under make check and make precheck, contract.ts byte-identical throughout (no wire type changes). Pin rig =0.45.0 until 0.45.x settles; the rust-version rises to 1.97.1 in S3.

Silent changes the slices must handle: from 0.43 Anthropic input tokens already include cache reads and writes, and Hotline adds them again (turn.rs every_input_token, the dispatcher's cache_beside_input), so both budgets over-charge and the context limit trips early; 0.43 puts the system prompt in the chat history and turn.rs overwrites the history every round, so the prompt would vanish; 0.45's own file store reads chatgpt.json/copilot.json, so Hotline must ship its own TokenStore in the same slice or every ChatGPT and Copilot user is signed out.

Not adopted: Rig 0.45's resumable runs and committed-history streaming for the Hotline Agent turn loop. Hotline writes into history mid-turn and never re-runs a tool after a restart; Rig re-runs unanswered calls and its saved-run format changes between releases. The exactly-once on_dispatch/on_outcome hook is adopted for voice metering (S6).

Runs before epic-21 (voice on the agent thread layer) touches the voice dispatcher and front: S1 rewrites the dispatcher's hooks. About 11 to 16 days; S1 and S3 are the critical path.

Open for George: adopt ChatGPT's browser sign-in (PKCE on 127.0.0.1:1455) now that Rig supports it; revoke tokens at the provider on credential delete (S5).
