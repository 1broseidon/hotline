---
id: task-52
title: Custom connections take effort levels, context limits and prices from models.dev by base URL
column: backlog
position: 35
priority: high
tags:
  - feature
  - 0.38.0
relatedFiles:
  - crates/hotline-core/src/models.rs
  - crates/hotline-core/src/driver/rig.rs
  - crates/hotline-core/src/pricing.rs
  - crates/hotline-core/src/providers/custom.rs
  - docs/sessions.md
subtasks:
  - id: task-52-1
    title: On custom save/refresh match base URL to a models.dev provider and store per-model efforts/limits/cost
    completed: false
  - id: task-52-2
    title: Resolve efforts/default_effort/output_limit/listed_price for custom-<id> ids from that stored metadata
    completed: false
  - id: task-52-3
    title: effort_params for CustomOpenAi by API (reasoning.effort vs reasoning_effort)
    completed: false
  - id: task-52-4
    title: "Headless harness: Meta-shaped connection offers Muse efforts and sends the chosen one; docs/sessions.md effort table + custom section"
    completed: false
createdAt: "2026-10-13T13:22:39.014Z"
updatedAt: "2026-10-14T20:05:30.131Z"
---

## Description
Custom OpenAI-compatible models get no Effort picker: efforts come from the bundled catalogue keyed by provider/model, and custom-<id> never matches; effort_params returns None for CustomOpenAi. Pricing falls back to UNPRICED ($5/$25 per M) — Meta Muse Spark 1.3 lists $1.25/$4.25, so its budget drains ~4-6x faster than the bill.

models.dev providers carry an `api` URL (meta: https://api.meta.ai/v1, exactly the user's connection). Bundling every endpoint provider adds ~1.4 MB (6x models.json), so look it up live instead: on custom save/refresh, fetch models.dev, match the base URL, store per-connection metadata (efforts, limits, cost) like discovery metadata. No match = today's behaviour.

Send: Responses -> {"reasoning":{"effort":e}}; Chat Completions -> {"reasoning_effort":e}.
