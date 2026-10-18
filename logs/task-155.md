---
id: task-155
title: "P2-46 (D): Search failures get a kind"
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:04.277Z"
relatedFiles:
  - crates/hotline-core/src/websearch.rs
  - crates/hotline-core/src/websearch/
updatedAt: "2026-10-18T03:02:55.748Z"
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T02:40:26.786Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T02:55:41.868Z"
    duration: 915
dependsOn:
  - task-126
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:16.610Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T02:40:12.004Z"
    ended: "2026-10-18T03:02:55.748Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T03:02:55.364Z"
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 46 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Search failures get a kind.** A local enum with status and retry-after picks the cooldown; the rendered text stays at the edge

- Source: errors F6, task 7
- Files: `websearch/mod.rs:108,193,687` and the adapters
- Collides with: 39
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T02:55:41.378Z: [claude] Done on dispatch/task-155 in one commit (0e907ef7). websearch::Failure now carries a `kind: Kind` (RateLimited, OutOfCredits, TimedOut, Status(u16), Resting, Other), set by the constructors the adapters already call (Failure::status, transport, new, and a new Failure::of); the adapters needed no edits. Failure::cooldown() matches on the kind (cooldown_for, which matched strings, is deleted), and the footer word is Kind::word (why() is deleted). Answered.failed is now Vec<(provider, Failure)>, so render reads the kind; the message is still scrubbed and rendered only at the edge, word for word. Behaviour change: a provider's body text that says "rate limited" or "timed out" (JSON-RPC errors, isError text, a 4xx detail) no longer rests the provider; only HTTP 429/402, timeouts and 5xx do. New test the_cooldown_follows_the_kind_whatever_the_words_say; the scripted test providers now return a typed Failure. make check and make precheck (7/7) pass. Row 39's lint autofix collides on websearch.rs. Nothing left unwired.
- 2026-10-18T03:02:54.987Z: Landed on release/0.40.0
