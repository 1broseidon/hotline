---
id: task-14
title: "A model the harness is too old for fails with a raw 400 for up to a day: refresh the ACP registry and say why"
column: backlog
position: 10
priority: high
tags:
  - bug
  - bro-176
createdAt: "2026-10-11T21:05:24.814Z"
description: |-
  ## What happened (30 Sep)

  A new Codex-backed teammate on George's desktop failed on its first turn with:

  ```
  {"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."}}
  ```

  The same model worked on Grizzly.

  ## Why

  * The ChatGPT backend gates models by the Codex client version: gpt-6.1-sol needs Codex ≥ 0.159. BRO-162 hit the same gate for model listing.
  * The ACP adapter version comes from the registry catalogue, cached at `cache/acp-registry.json` with a 24h TTL (`driver/acp/registry.rs` `CACHE_TTL_MS`).
  * The cache is refreshed only when the picker opens (`backends()`). Starting a session reads the cache without touching the network (`cached_backends()`).
  * The desktop's cache was from 15:10 on 29 Sep, which named `codex-acp` 2.0.0. That depends on `@openai/codex ^0.158.0`, and because `^` on a 0.x version pins the minor, it resolved to 0.158.0.
  * `codex-acp` 2.0.1 (`^0.159.1`) was published afterwards. Grizzly fetched the catalogue later and got it.
  * So the desktop keeps starting a Codex too old for the model the picker offers until the cache ages out, up to a day.
  * Deleting the cache doesn't help either: session start then falls back to the hand-pinned `codex-acp@1.12.0` (Codex 0.154) until the picker is opened.

  This was worked around by hand: the cache was rewritten from the live registry and the session restarted.

  ## Fix

  1. **Refresh on this error.** When an ACP turn fails with a model-not-supported error (400, or `not supported when using Codex`, or the Claude adapter's equivalent), refresh the catalogue ignoring the TTL. If that yields a newer adapter version for that backend, restart the session once on it and retry the turn.
  2. **Say why when it can't recover.** Show a card like "gpt-6.1-sol needs a newer Codex than this desk has (0.158). Hotline couldn't find a newer one yet; try again later or pick another model," not the raw JSON.
  3. **Refresh sooner.** Check for a newer catalogue in the background at app start and every few hours (the fetch is one small GET). Keep the cache as the offline fallback. Session start still never waits on the network.
  4. **Never go backwards.** When the cache is missing or unreadable, start with the newest adapter version seen before (keep a last-known-good copy), not the hand-pinned one.

  ## Done when

  * A scripted ACP backend that answers the first turn with the not-supported error, where the refreshed catalogue has a newer version, retries on the new version without the person doing anything.
  * With no newer version, the person sees the plain-language card.
  * A test covers a missing cache not downgrading below the last-known-good version.

  Migrated from Linear BRO-176: https://linear.app/1broseidon/issue/BRO-176/a-model-the-harness-is-too-old-for-fails-with-a-raw-400-for-up-to-a (2026-10-11).
updatedAt: "2026-10-14T20:05:17.923Z"
---
