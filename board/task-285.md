---
id: task-285
title: "Model picker: accounts, then models, then efforts, with an effort slider in the head"
column: todo
position: 6
priority: high
tags:
  - accounts
  - picker
  - ui
parentId: epic-25
createdAt: "2026-10-24T06:23:17.213Z"
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
