---
id: task-285
title: "Model picker: accounts, then models, then efforts, with an effort slider in the head"
column: review
position: 6
priority: high
tags:
  - accounts
  - picker
  - ui
parentId: epic-25
createdAt: "2026-10-24T06:23:17.213Z"
updatedAt: "2026-10-24T07:04:42.543Z"
---

## Description
S2. Mock: mockups/picker/index.html (approved, slider version).

Done when:
- The button reads model · effort with the maker's mark; the account is in the tooltip, not on the button. (UI test)
- The menu head names the current model and its account with an effort slider, one stop per level the model has; moving it changes effort only. (UI test)
- One row per account: 'Provider' when it has one account, 'Provider · Name' when two or more; the account in use has the dot. (UI test)
- An account opens its models with makers' marks; a model opens its efforts; keyboard can walk all three levels. (UI test)
- Clicking a model keeps the current effort when the model has it, else the model's default; clicking an effort sets both. (UI test)
- Add an account… opens Settings › Providers. (UI test)
- Brand marks are shown only as images (assets/marks, thesvg MIT, README there), never as markup.

## Log
- 2026-10-24T07:04:41.959Z: [claude] Overnight 2026-10-24, feat/accounts 7f2eda69 + b4cb3c4a. Done as approved in mockups/picker: button model · effort with maker mark (thesvg marks, MIT, assets/marks/README), head names model + account with an effort slider (one stop per level), one row per account with the in-use dot, account opens models, model opens efforts, Add an account opens Settings › Providers, keyboard walks the three levels. Checked live on the Linux computer with two Anthropic accounts (screens in the overnight report). Tests: tests/model-picker.test.tsx (10).
