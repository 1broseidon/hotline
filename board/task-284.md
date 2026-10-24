---
id: task-284
title: "Settings › Providers: add another account of a provider, name and rename accounts"
column: todo
position: 5
priority: high
tags:
  - accounts
  - settings
  - ui
parentId: epic-25
createdAt: "2026-10-24T06:23:16.654Z"
---

## Description
S1.

Done when:
- A connected provider can be added again; the add list no longer hides it (Providers.tsx:93-94). (UI test)
- A new account asks for a name, prefilled 'Provider 2'; any account can be renamed. (UI test + wire test)
- Each provider lists all its accounts with name and kind (subscription, API key, this Mac). (UI test)
- Adding a second account changes nothing about the first: its secret, model list and manual models stay, and teammates on it stay. (vault test)
- Sign in again replaces only the account it was started from.

## Log
- 2026-10-24T07:04:40.815Z: [claude] Overnight 2026-10-24, feat/accounts 8ed7bf43. Done: Add provider lists every provider (a connected one can be added again); a new account of a provider already connected starts named 'Provider 2'; rows and picker name accounts 'Provider · Name' when 2+; account pages rename via new credential.rename (vault.rename, owner desk only; phone seat refuses it). Tests: tests/provider-accounts.test.tsx, desk harness rename asserts. NOT done: a provider page's model filter, manual models and refresh still act by provider id (the default account), so a second account's own model list cannot be edited yet.
