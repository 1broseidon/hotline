---
id: task-273
title: "Desk: offer your own browser's cookie for the site a teammate opens, always asking, and pull it back out of the container on demand"
column: in-progress
position: 0
priority: high
tags:
  - credentials
  - cookies
parentId: epic-24
createdAt: "2026-10-22T15:31:06.921Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T15:31:13.865Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59533
    at: "2026-10-22T15:31:16.600Z"
assignee: codex
updatedAt: "2026-10-22T15:31:52.219Z"
---

## Description
George, 2026-10-22 10:26–10:30. When a teammate's computer opens a site that one of the person's host browsers (Brave, Chrome, Edge, Chromium, Firefox) holds cookies for, the desk raises a card: 'Use your existing cookie from Brave for porkbun.com?', with Use, Not now and Never for this site, on the desk and the phone. There is no automatic mode: every lend is asked. The card says plainly that the teammate will be signed in as you.

Detection reads only which sites a browser has cookies for, without decrypting anything or launching the browser: copy the profile's Cookies file and read host_key (Chromium) or the plaintext moz_cookies host (Firefox). Values are read only after Use, through the existing path (crates/hotline-core/src/computer/cookies.rs: the headless browser on a profile copy over CDP Network.getAllCookies), filtered to that site's registrable domain and its subdomains, and pushed with the existing computer.cookies push (computer/login.rs) under a name that says where they came from (for example lent-brave-default).

Revocation is in the container only (George: the cookie itself is not revoked; access inside the container is what matters). The settled card and the teammate's computer pane show 'Signed in as you on porkbun.com, from Brave · Remove'. Remove calls the computer's existing DELETE /logins/{name} with {"domains": [site]}, and the agent is told the session was taken back. No extension change.

Asked once per site per session; Never for this site is remembered per teammate and can be undone in Settings. Sites come from the computer's GET /pages (task for hotline-computer, below); the desk polls it with the use door every 2 s and asks for an origin only when it first appears.

Done when:
- A site with host cookies raises one card, on desk and phone, and none for a site without them: named test with a fixture Chromium and Firefox store.
- Detection never decrypts or launches a browser: named test asserting no launch and no keychain call until Use.
- Use pushes only that site's cookies (registrable domain and subdomains) and the agent hears it may reload: named test.
- Not now asks again next session; Never for this site never asks again for that teammate until undone: named tests.
- Remove drops the site's cookies from the computer through DELETE /logins/{name} and says so to the agent: named test against the computer fake.
- No cookie value reaches the tape, the agent or logs: sink test.
- Manual check: porkbun.com signed in from Brave on George's Mac, then Remove signs the teammate out.

## Log
- 2026-10-22T15:40:03.080Z: [codex] Implemented host-only site discovery, PSL-bounded explicit lends, durable removal records, shared card/phone handlers and Settings undo. Task-274's /pages response shape is confirmed; older computers return no offers. Now adding real-handler/fake-computer and UI proofs.
