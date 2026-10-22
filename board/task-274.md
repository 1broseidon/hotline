---
id: task-274
title: "Computer (hotline-computer): GET /pages lists the sites its browser has open, for the desk's cookie offer"
column: review
position: 2
priority: high
tags:
  - credentials
  - cookies
  - hotline-computer
parentId: epic-24
createdAt: "2026-10-22T15:31:07.467Z"
updatedAt: "2026-10-22T15:36:54.286Z"
---

## Description
In ~/Projects/personal/hotline-computer (ext/v3, rc.8). A bearer door, GET /pages, answers {"pages": [{"origin": "https://porkbun.com", "url": "https://porkbun.com/account", "title": "…", "openedAt": <ms>}]} for each top-level page tab of the managed Chromium (from CDP target info), web origins only (http/https), newest first. No extension change. 404 from an older computer tells the desk there is no offer.

Done when:
- The door lists the open top-level pages and drops a closed one: contract test (tests/contract.rs).
- Non-web targets (chrome://, the extension, devtools) are never listed: unit test.
- make check passes; image built as rc.8 and the door checked against it.

## Log
- 2026-10-22T15:36:53.783Z: [claude] Built as rc.8 on hotline-computer ext/v3 (3b9e527, not pushed). GET /pages answers {"pages":[{origin,url,title,openedAt}]}, bearer-only, http/https page targets only, newest first; read from DevTools /json/list on 127.0.0.1:9222 so the desk's poll never waits behind a running browser action; {"pages":[]} when no browser runs; 502 on a DevTools error; 404 from older computers. make check 152 tests; contract test green against hotline-computer:0.11.0-rc.8.
