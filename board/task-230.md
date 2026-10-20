---
id: task-230
title: "S0: narrow Hotline's Rig seam and pin today's behaviour (still on 0.42)"
column: todo
position: 2
priority: high
tags:
  - rig
assignee: claude
relatedFiles:
  - docs/plans/rig-0.45-migration.md
  - crates/hotline-core/src/driver/rig.rs
  - crates/hotline-core/src/driver/rig/turn.rs
  - crates/hotline-core/src/driver/rig/build.rs
  - crates/hotline-core/src/driver/rig/recovery.rs
  - crates/hotline-core/src/driver/failure.rs
  - crates/hotline-core/src/pricing.rs
  - crates/hotline-core/src/voice/dispatcher.rs
parentId: epic-22
createdAt: "2026-10-20T06:51:42.743Z"
updatedAt: "2026-10-20T12:02:25.036Z"
---

## Description
Report §6 S0. No dependency bump: Rig stays 0.42.0 and Cargo.lock's rig entry is unchanged.

1. Copy the research report from `/Users/george/Library/Application Support/Hotline/workspaces/c9397b69-9084-4b3b-a325-715134487a53/rig-045-migration.md` into `docs/plans/rig-0.45-migration.md`. Add one paragraph at the top saying the plan changed on 2026-10-20: S1 now jumps straight from 0.42 to 0.45 (the report's S1+S2+S3 in one slice; S2 and S3 retired), and that `/tmp/rig-*` paths in the report mean Rig's source at those tags. Otherwise leave the report as written.
2. Narrow the seam (behaviour-preserving refactors only): one usage adapter (Rig `Usage` → Hotline counters, `TokenUsage`, pricing input) so 0.43's `Option` change touches one function; one stream-event mapper in `driver/rig/turn.rs` (text, thought, final); the preamble kept as a `String` on `Turn` and prepended per round instead of `CompletionRequest.preamble`; `driver/rig/build.rs` splits `completion_model(..)` from `agent_builder(..)` where 0.42 allows.
3. Behaviour-pinning tests that must survive the bump. Name every one with the prefix `rig_pin_` so later slices can find them. Pin at the HTTP wire (request bodies captured by a local stub server) and on Hotline's own counters, never on Rig types, so S1 can keep their assertions and expected values byte-for-byte and change at most how a fixture is built. At least: Anthropic usage totals with cache reads/writes (cache tokens priced once, budgets and the context threshold unchanged); request-body snapshots for cache markers (Anthropic `cache_control`, OpenRouter `anthropic/`); the system prompt present in every round's request body of a multi-round turn; Copilot chat assistant content sent as a string; Ollama `think` reaching `/api/chat`; the `Failure::classify` table on real recorded error shapes.

contract.ts stays byte-identical.
