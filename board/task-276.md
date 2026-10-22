---
id: task-276
title: "Cookie offer: say once when macOS won't let the desk read your browsers, instead of offering nothing"
column: todo
position: 2
priority: high
tags:
  - bug
  - cookies
  - macos
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/session/cookie_offers.rs
  - crates/hotline-core/src/computer/cookies/lending.rs
parentId: epic-24
createdAt: "2026-10-22T16:27:07.323Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T16:27:15.808Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T16:27:15.655Z"
  until: review
  tries: 0
assignee: codex
---

## Description
George's QA, 2026-10-22 11:25: no card on porkbun.com in the dev window though Brave holds 18 porkbun cookies. Temporary logging showed the desk saw porkbun.com from GET /pages, then lending::discover() returned no sources at all. macOS keeps an app out of another app's data without Full Disk Access (the comment on Candidate::refused in computer/cookies.rs says so): listing the profile folder fails with Operation not permitted, profiles() is empty, and the offer path drops the site silently. It is then marked seen, so it is not tried again that session.

Fix:
- In look_at_cookie_offers, when a candidate that is installed and has a data folder is refused (Candidate::refused), and a site was otherwise eligible, write one notice in the teammate's chat, once per desk run per browser: 'Hotline can't read Brave's cookies, so it can't offer your session for porkbun.com. Allow Hotline in System Settings → Privacy & Security → Full Disk Access, then reopen Hotline.' Name the app as macOS lists it (the dev binary in a dev build).
- Do not mark a site seen when discovery was refused, so it is offered once access is granted (after the restart macOS needs).
- Same wording in Settings, beside the existing blocked state of the cookie import picker.

Done when:
- A refused browser raises the notice once and no offer; a granted one offers: named tests with a refusing fixture.
- The site is offered after access appears, without reopening the page: named test.
- Manual check on George's Mac: without Full Disk Access the notice shows; with it, the porkbun.com card appears.
