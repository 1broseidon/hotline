---
id: task-286
title: Voice, image generation and chapter notes follow the teammate's account
column: review
position: 7
priority: medium
tags:
  - accounts
  - voice
  - imagegen
  - chapters
parentId: epic-25
createdAt: "2026-10-24T06:23:17.773Z"
updatedAt: "2026-10-24T07:04:43.675Z"
---

## Description
S6.

Done when:
- A teammate's chapter notes run on its account when that account serves the note model. (chapters test)
- Image generation and voice for a teammate prefer the teammate's account for their provider; room-level work with no teammate uses the provider's default account. (imagegen + voice tests)
- No background work for one teammate spends another account's quota: a two-account test proves each call's key.

## Log
- 2026-10-24T07:04:43.106Z: [claude] Overnight 2026-10-24, feat/accounts 9bd0fb52. Done: chapter notes run on the teammate's account (note_model + Agents::complete through account_keys); generate_image draws on the teammate's account for its provider (imagegen::resolve_on, persona accounts_of); a call's completion on a teammate's model uses its account (voice room completion_builder); voice pricing bills the named account. NOT done: speech-to-text/text-to-speech providers (voice/connected.rs) still use each provider's default account; no two-account test proves each call's key at the HTTP level yet.
