---
id: task-277
title: "Settings: cookie offers on or off (on by default), for all sites or a selected list, like a provider's model filter"
column: review
position: 3
priority: high
tags:
  - cookies
  - settings
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/session/cookie_offers.rs
  - ui/src/components/settings/Computer.tsx
  - ui/src/components/settings/Providers.tsx
parentId: epic-24
createdAt: "2026-10-22T21:03:38.914Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T21:23:17.535Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T21:03:40.273Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 66466
    at: "2026-10-22T21:03:43.170Z"
    ended: "2026-10-22T21:23:17.535Z"
    outcome: review
assignee: codex
decisions:
  - decision-39
---

## Description
George, 2026-10-22 16:02. A room setting for task-273's cookie offers, modelled on the enabledModels filter (ui/src/components/settings/Providers.tsx modelsShownText and ProviderPage; contract session.rs enabledModels):

- cookieOffers: {enabled: bool (default true), sites?: [registrable domain]}. sites absent means all sites; a list means only those. Ticking every site removes the list rather than writing one that means all.
- Settings → Computer: a switch 'Offer your browser cookies' and a row 'Sites: All sites | N sites' that opens a page listing the sites the person's browsers hold cookies for (from lending discovery: registrable domain plus which browser and profile), searchable, a tick per site, Save and Back, as the provider page does. The same page lists the 'Never for this site' blocks (computer_cookie_blocked) with Undo.
- Off means off: look_at_cookie_offers returns before discovery, so no browser data is read, not even site names; pending offers expire with a line saying offers are off. The list page reads site names only when the person opens it.
- Desk and owner seats change it; the phone may read it.
- The page notes that account-wide sites (google.com, apple.com, microsoft.com) lend the whole account.

Done when:
- Off: no discovery runs and pending offers expire: named test (a discovery fixture that records calls).
- A selected list offers only its sites; all sites offers any: named tests.
- Ticking every site saves no list: named test, as with models.
- The setting survives restart and a never-for-this-site block can be undone from the page: named tests.
- UI tests for the switch, the row's words (All sites / 1 site / N sites) and the page.
- make check passes.

## Log
- 2026-10-22T21:05:50.253Z: [codex] Read cookie lending decisions and traced discovery, answer, settings and seats. Implementing a typed default-on room setting; a narrow phone read; owner/desk-only on-demand site discovery; immediate pending-card expiry on settings writes, serialized with discovery and answers through the existing policy gate.
- 2026-10-22T21:15:49.397Z: [codex] Implemented room cookieOffers, phone preference read, on-demand grouped browser/profile sites, Computer switch/Sites page, filtering and immediate pending expiry, Undo and account-wide wording. All 15 cookie-offer core tests, 2 restart harness tests and 6 UI tests pass. Full check required patching transitive DOMPurify 3.4.16→3.4.17 (committed separately as f1fd6262); fixing the one new Clippy finding and rerunning required checks.
- 2026-10-22T21:21:17.710Z: [codex] make check passes on dispatch/task-277: full UI/type/lint/dependency gates, Python checks, Rust fmt/Clippy/workspace tests and named cookie/restart tests. Commits f1fd6262, 4abd42c6 and 9c45e19d; working tree clean. Running make precheck for the review hand-off.
- 2026-10-22T21:23:11.152Z: [codex] Ready for review on dispatch/task-277 (HEAD 9c45e19d; feature 4abd42c6; dependency patch f1fd6262). Added default-on cookieOffers settings, automatic discovery/answer enforcement and pending expiry, on-demand searchable Computer → Sites with browser/profile sources, all/selected normalization, Undo and account-wide wording. Named core, restart and UI tests pass; make check and all 8 make precheck legs pass, including Windows and isolated UI files. Working tree clean; nothing pushed or merged. Next action: review and merge the branch; task remains for a person to complete.
